import React from "react";
export type StatusDotState = "idle" | "in_progress" | "needs_attention" | "error" | "completed";
export interface StatusDotProps {
    state: StatusDotState;
    size?: "small" | "medium";
    tooltipText?: string;
}
export declare const StatusDot: React.FC<StatusDotProps>;
