import React from "react";
import type { ToolCall } from "../../services/refact/types";
import { type AgentStatusReport } from "./AgentStatusModel";
type AgentStatusContentProps = {
    report: AgentStatusReport;
    onSubmitCommand?: (command: string) => void | Promise<void>;
    actionsDisabled?: boolean;
};
type AgentStatusViewProps = {
    toolCall: ToolCall;
};
export declare const AgentStatusContent: React.FC<AgentStatusContentProps>;
export declare const AgentStatusView: React.FC<AgentStatusViewProps>;
export default AgentStatusView;
