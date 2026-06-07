import React from "react";
import type { BuddyPetState, Stage } from "./types";
interface StatsSummaryData {
    totals: {
        total_calls: number;
        successful_calls: number;
        total_tokens: number;
    };
}
interface BuddySummaryStripProps {
    stage: Stage;
    xp: number;
    xpNext: number | undefined;
    xpFill: number;
    pet: BuddyPetState | undefined;
    statsData: StatsSummaryData | undefined;
    successRate: number | null;
    onViewStats: () => void;
}
export declare const BuddySummaryStrip: React.FC<BuddySummaryStripProps>;
export {};
