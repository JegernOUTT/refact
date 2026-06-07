import React from "react";
import type { ToolCall } from "../../services/refact/types";
import { type AgentDiffReport } from "./AgentDiffModel";
type AgentDiffContentProps = {
    report: AgentDiffReport;
};
type AgentDiffViewProps = {
    toolCall: ToolCall;
};
export declare const AgentDiffContent: React.FC<AgentDiffContentProps>;
export declare const AgentDiffView: React.FC<AgentDiffViewProps>;
export default AgentDiffView;
