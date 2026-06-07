import type { HistoryTreeNode } from "../../../History/historySlice";
export type TrailDot = {
    id: string;
    chatId: string;
    type: "user" | "assistant" | "subagent" | "fork" | "active" | "completed";
    label?: string;
    depth: number;
    hasBranch: boolean;
};
export declare function buildDotTrail(node: HistoryTreeNode, maxDots?: number): TrailDot[];
