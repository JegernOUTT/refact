import { LspChatMode, ReasoningEffort } from "../features/Chat/Thread/types";
import { SystemPrompts } from "../services/refact/prompts";
export interface PersistedModeParams {
    model?: string;
    boost_reasoning?: boolean;
    reasoning_effort?: ReasoningEffort;
    thinking_budget?: number;
    temperature?: number;
    frequency_penalty?: number;
    max_tokens?: number;
    parallel_tool_calls?: boolean;
    increase_max_tokens?: boolean;
    include_project_info?: boolean;
    system_prompt?: SystemPrompts;
    checkpoints_enabled?: boolean;
    follow_ups_enabled?: boolean;
}
export interface PersistedThreadParams extends PersistedModeParams {
    mode?: LspChatMode;
}
export declare function saveModeParams(mode: LspChatMode, params: Partial<PersistedModeParams>): void;
export declare function getModeParams(mode: LspChatMode): Partial<PersistedModeParams>;
export declare function getLastThreadParams(mode?: LspChatMode): Partial<PersistedThreadParams>;
export declare function saveLastThreadParams(params: Partial<PersistedThreadParams>): void;
export declare function saveDraftMessage(threadId: string, content: string): void;
export declare function getDraftMessage(threadId: string): string;
export declare function clearDraftMessage(threadId: string): void;
export declare function clearAllDraftMessages(): void;
export declare function pruneStaleDraftMessages(): void;
