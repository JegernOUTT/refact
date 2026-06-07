import React from "react";
import type { HistoryTreeNode } from "../../../History/historySlice";
import type { DashboardBreakpoint } from "../../types";
type RecentItemProps = {
    node: HistoryTreeNode;
    breakpoint: DashboardBreakpoint;
    depth: number;
    isExpanded: boolean;
    onToggleExpand: (id: string) => void;
    onClick: () => void;
    onDotClick?: (chatId: string) => void;
    onDelete?: (id: string) => void;
    onRename?: (id: string, newTitle: string) => void;
};
export declare const RecentItem: React.FC<RecentItemProps>;
export {};
