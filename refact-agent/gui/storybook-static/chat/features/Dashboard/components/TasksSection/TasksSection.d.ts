import React from "react";
import type { DashboardBreakpoint } from "../../types";
type TasksSectionProps = {
    breakpoint: DashboardBreakpoint;
    collapsed: boolean;
    projectLoading: boolean;
    loadError: string | null;
    onToggleCollapsed: () => void;
};
export declare const TasksSection: React.FC<TasksSectionProps>;
export {};
