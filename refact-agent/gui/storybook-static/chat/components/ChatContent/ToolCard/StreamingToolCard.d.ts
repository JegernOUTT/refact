import React from "react";
import { ToolCall } from "../../../services/refact/types";
interface StreamingToolCardProps {
    toolCall: ToolCall;
    icon: React.ReactNode;
    summary: React.ReactNode;
    meta?: string | null;
}
export declare const StreamingToolCard: React.FC<StreamingToolCardProps>;
export default StreamingToolCard;
