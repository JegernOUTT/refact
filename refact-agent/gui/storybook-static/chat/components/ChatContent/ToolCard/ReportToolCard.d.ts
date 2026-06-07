import React from "react";
import { ToolCall } from "../../../services/refact/types";
export type ReportVariant = "taskDone" | "plan" | "report";
export interface ReportData {
    summary?: string;
    markdown: string;
    filesChanged?: string[];
    knowledgePath?: string;
}
interface ReportToolCardProps {
    toolCall: ToolCall;
    icon: React.ReactNode;
    defaultSummary: React.ReactNode;
    variant?: ReportVariant;
    meta?: string | null;
    extractReport?: (content: string) => ReportData | null;
    defaultOpen?: boolean;
    unboundedContent?: boolean;
}
export declare const ReportToolCard: React.FC<ReportToolCardProps>;
export default ReportToolCard;
