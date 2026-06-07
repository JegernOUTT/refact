import React from "react";
import type { OpenTabData, DashboardBreakpoint } from "../../types";
type OpenTabCardProps = {
    tab: OpenTabData;
    breakpoint: DashboardBreakpoint;
    modeLabel?: string;
    onClick: () => void;
};
export declare const OpenTabCard: React.FC<OpenTabCardProps>;
export {};
