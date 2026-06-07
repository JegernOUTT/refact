import React from "react";
import type { ToolCall } from "../../services/refact/types";
import { type AgentPulseReport } from "./AgentPulseModel";
type AgentPulseContentProps = {
    report: AgentPulseReport;
    onSubmitCommand?: (command: string) => void | Promise<void>;
    actionsDisabled?: boolean;
};
type AgentPulseViewProps = {
    toolCall: ToolCall;
};
export declare const AgentPulseContent: React.FC<AgentPulseContentProps>;
export declare const AgentPulseView: React.FC<AgentPulseViewProps>;
export default AgentPulseView;
