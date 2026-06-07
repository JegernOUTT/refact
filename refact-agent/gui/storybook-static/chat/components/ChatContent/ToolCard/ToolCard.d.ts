import React from "react";
import { ToolCall } from "../../../services/refact/types";
export type ToolStatus = "running" | "success" | "error";
export interface ToolCardProps {
    icon: React.ReactNode;
    summary: React.ReactNode;
    meta?: React.ReactNode;
    status: ToolStatus;
    isOpen: boolean;
    onToggle: () => void;
    children?: React.ReactNode;
    className?: string;
    animate?: boolean;
    toolCall?: ToolCall;
}
export declare const ToolCard: React.NamedExoticComponent<ToolCardProps>;
export default ToolCard;
