import type { ToolCall } from "../services/refact/types";
export declare function normalizeToolName(name: string | undefined): string | undefined;
export declare function normalizeToolCall(toolCall: ToolCall): ToolCall;
export declare function isToolName(name: string | undefined, expectedName: string): boolean;
export declare function formatToolDisplayName(name: string | undefined): string;
