import React from "react";
import type { OpenTabData, DashboardBreakpoint } from "../../types";
type OpenSectionProps = {
    tabs: OpenTabData[];
    breakpoint: DashboardBreakpoint;
    collapsed: boolean;
    onToggleCollapsed: () => void;
};
export declare const OpenSection: React.FC<OpenSectionProps>;
export {};
