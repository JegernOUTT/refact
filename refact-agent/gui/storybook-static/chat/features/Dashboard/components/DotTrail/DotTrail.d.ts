import React from "react";
import type { HistoryTreeNode } from "../../../History/historySlice";
import type { DashboardBreakpoint } from "../../types";
type DotTrailProps = {
    node: HistoryTreeNode;
    breakpoint: DashboardBreakpoint;
    onDotClick?: (chatId: string) => void;
};
export declare const DotTrail: React.FC<DotTrailProps>;
export {};
