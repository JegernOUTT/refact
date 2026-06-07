import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { ChatMessage, DiffChunk, ToolCall } from "./types";
export declare const toolsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getToolGroups: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolGroup[], "tools">;
    updateToolGroups: MutationDefinition<ToolGroupUpdate[], BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", {
        success: true;
    }, "tools">;
    checkForConfirmation: MutationDefinition<ToolConfirmationRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolConfirmationResponse, "tools">;
    dryRunForEditTool: MutationDefinition<{
        toolName: string;
        toolArgs: Record<string, unknown>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolEditResult, "tools">;
}, "tools", "TOOL_GROUPS", typeof coreModuleName | typeof reactHooksModuleName>;
export type ToolGroupUpdate = {
    name: string;
    source: ToolSource;
    enabled: boolean;
};
export type ToolGroup = {
    name: string;
    category: "integration" | "mcp" | "builtin";
    description: string;
    tools: Tool[];
};
export type ToolSource = {
    source_type: "builtin" | "integration";
    config_path: string;
};
export type ToolSpec = {
    name: string;
    display_name: string;
    description: string;
    input_schema: Record<string, unknown>;
    output_schema?: Record<string, unknown>;
    annotations?: {
        title?: string;
        readOnlyHint?: boolean;
        destructiveHint?: boolean;
        idempotentHint?: boolean;
        openWorldHint?: boolean;
    };
    source: ToolSource;
    agentic: boolean;
    experimental?: boolean;
    allow_parallel?: boolean;
};
export type Tool = {
    spec: ToolSpec;
    enabled: boolean;
};
export type ToolConfirmationPauseReason = {
    type: "confirmation" | "denial" | "unknown";
    raw_type?: string;
    tool_name: string;
    command: string;
    rule: string;
    tool_call_id: string;
    integr_config_path: string | null;
};
export type ToolConfirmationResponse = {
    pause: boolean;
    pause_reasons: ToolConfirmationPauseReason[];
};
export type ToolConfirmationRequest = {
    tool_calls: ToolCall[];
    messages: ChatMessage[];
};
export declare function isToolGroup(tool: unknown): tool is ToolGroup;
export declare function isToolConfirmationResponse(data: unknown): data is ToolConfirmationResponse;
export type ToolEditResult = {
    file_before: string;
    file_after: string;
    chunks: DiffChunk[];
};
export declare function isToolEditResult(data: unknown): data is ToolEditResult;
