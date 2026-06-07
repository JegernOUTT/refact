import React from "react";
interface BuddyRecentChatsProps {
    compact?: boolean;
    maxItems?: number;
    showFilters?: boolean;
    onViewAll?: () => void;
    title?: string;
    className?: string;
}
export declare const BuddyRecentChats: React.FC<BuddyRecentChatsProps>;
export {};
