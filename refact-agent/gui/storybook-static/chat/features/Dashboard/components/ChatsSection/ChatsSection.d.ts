import React from "react";
import type { DashboardBreakpoint } from "../../types";
type ChatsSectionProps = {
    breakpoint: DashboardBreakpoint;
    collapsed: boolean;
    projectLoading: boolean;
    onToggleCollapsed: () => void;
};
export declare const ChatsSection: React.FC<ChatsSectionProps>;
export {};
