import type { ToolCall, ToolResult } from "../../../services/refact/types";
import type { ToolStatus } from "./ToolCard";
export declare function toolNameLabel(name: string): string;
export type OpenAiResponsesToolCardState = {
    toolName: string;
    label: string;
    isOpen: boolean;
    toggleOpen: () => void;
    status: ToolStatus;
    parsedArgs: unknown;
    rawJson: string;
    maybeResult: ToolResult | undefined;
    contentText: string | null;
};
export declare function useOpenAiResponsesToolCardState(toolCall: ToolCall): OpenAiResponsesToolCardState;
