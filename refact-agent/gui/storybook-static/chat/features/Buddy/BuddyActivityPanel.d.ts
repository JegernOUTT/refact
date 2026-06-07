import React from "react";
import type { BuddyActivityEntry } from "./types";
interface BuddyActivityPanelProps {
    activities: BuddyActivityEntry[];
    onOpenChat?: (chatId: string, title: string) => void;
}
export declare const BuddyActivityPanel: React.FC<BuddyActivityPanelProps>;
export {};
