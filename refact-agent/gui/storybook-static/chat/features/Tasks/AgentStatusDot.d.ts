import React from "react";
interface AgentStatusDotProps {
    status: "doing" | "done" | "failed";
    size?: "small" | "medium";
}
export declare const AgentStatusDot: React.FC<AgentStatusDotProps>;
export {};
