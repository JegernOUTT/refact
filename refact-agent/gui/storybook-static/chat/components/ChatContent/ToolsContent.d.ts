import React from "react";
import { ChatContextFile, DiffChunk, ToolCall } from "../../services/refact";
export declare const SingleModelToolContent: React.FC<{
    toolCalls: ToolCall[];
}>;
export type ToolContentProps = {
    toolCalls: ToolCall[];
    contextFilesByToolId?: Record<string, ChatContextFile[]>;
    diffsByToolId?: Record<string, DiffChunk[]>;
    isActiveAssistant?: boolean;
};
export declare const ToolContent: React.FC<ToolContentProps>;
