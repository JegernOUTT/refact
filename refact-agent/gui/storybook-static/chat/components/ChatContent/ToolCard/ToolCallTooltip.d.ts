import React from "react";
import type { ToolCall } from "../../../services/refact/types";
interface ToolCallTooltipProps {
    toolCall: ToolCall;
    children: React.ReactNode;
}
export declare const ToolCallTooltip: React.FC<ToolCallTooltipProps>;
export {};
