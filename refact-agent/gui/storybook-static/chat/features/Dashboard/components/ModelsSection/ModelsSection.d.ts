import React from "react";
import type { DashboardBreakpoint } from "../../types";
type ModelsSectionProps = {
    breakpoint: DashboardBreakpoint;
    collapsed: boolean;
    onToggleCollapsed: () => void;
};
export declare const ModelsSection: React.FC<ModelsSectionProps>;
export {};
