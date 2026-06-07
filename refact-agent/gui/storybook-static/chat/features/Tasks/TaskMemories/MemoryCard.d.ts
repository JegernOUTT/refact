import React from "react";
import type { TaskMemoryEntry } from "../../../services/refact/taskMemoriesApi";
type MemoryCardProps = {
    memory: TaskMemoryEntry;
    onPin: (filename: string, pinned: boolean) => void | Promise<void>;
    onArchive: (filename: string) => void | Promise<void>;
    disabled?: boolean;
    pending?: boolean;
    expanded?: boolean;
    onExpandedChange?: (filename: string, expanded: boolean) => void;
};
export declare const MemoryCard: React.FC<MemoryCardProps>;
export default MemoryCard;
