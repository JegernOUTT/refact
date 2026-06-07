import React from "react";
import type { BuddyRuntimeEvent } from "./types";
export type RecentBuddyError = BuddyRuntimeEvent & {
    occurrences?: number;
    dismissedAny?: boolean;
    dismissedAll?: boolean;
    relatedIds?: string[];
};
interface BuddyRecentErrorsPanelProps {
    recentErrors: RecentBuddyError[];
    onInvestigate: (event: RecentBuddyError) => void | Promise<void>;
    onDismiss: (event: RecentBuddyError) => void | Promise<void>;
}
export declare const BuddyRecentErrorsPanel: React.FC<BuddyRecentErrorsPanelProps>;
export {};
