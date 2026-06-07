import { ChatRole, ThinkingBlock, ToolCall, ToolResult, UserMessage } from "./types";
export type LspChatMessage = {
    role: ChatRole;
    content: string | null;
    finish_reason?: "stop" | "length" | "abort" | "tool_calls" | "error" | null;
    thinking_blocks?: ThinkingBlock[];
    tool_calls?: ToolCall[];
    tool_call_id?: string;
    usage?: Usage | null;
    extra?: Record<string, unknown>;
} | UserMessage | {
    role: "tool";
    content: ToolResult["content"];
    tool_call_id: string;
    extra?: Record<string, unknown>;
};
export declare function isLspChatMessage(json: unknown): json is LspChatMessage;
export declare function isLspUserMessage(message: LspChatMessage): message is UserMessage;
export type CompletionTokenDetails = {
    accepted_prediction_tokens: number | null;
    audio_tokens: number | null;
    reasoning_tokens: number | null;
    rejected_prediction_tokens: number | null;
};
export type PromptTokenDetails = {
    audio_tokens: number | null;
    cached_tokens: number;
};
export type MeteringUsd = {
    prompt_usd: number;
    generated_usd: number;
    cache_read_usd?: number;
    cache_creation_usd?: number;
    total_usd: number;
};
export type Usage = {
    completion_tokens: number;
    prompt_tokens: number;
    total_tokens: number;
    completion_tokens_details?: CompletionTokenDetails | null;
    prompt_tokens_details?: PromptTokenDetails | null;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
    cache_creation_tokens?: number;
    cache_read_tokens?: number;
    metering_usd?: MeteringUsd;
};
export type TokenMapSegment = {
    label: string;
    category: string;
    tokens: number;
    percentage: number;
};
export type TokenMapItem = {
    category: string;
    label: string;
    tokens: number;
};
export type TokenMap = {
    total_prompt_tokens: number;
    max_context_tokens: number;
    estimated: boolean;
    segments: TokenMapSegment[];
    top_items: TokenMapItem[];
};
