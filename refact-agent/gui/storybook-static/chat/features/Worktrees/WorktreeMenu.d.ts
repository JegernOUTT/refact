import React from "react";
import { type WorktreeMeta, type WorktreeRecordView } from "../../services/refact";
type WorktreeMenuProps = {
    currentWorktree: WorktreeMeta | null;
    currentRecord?: WorktreeRecordView | null;
    records: WorktreeRecordView[];
    isLoading: boolean;
    feedback?: string | null;
    canCopyPath: boolean;
    onCreate: () => void;
    onSelect: (record: WorktreeRecordView) => void;
    onDetach: () => void;
    onOpenInNewWindow: () => void;
    onCopyPath: () => void;
};
export declare const WorktreeMenu: React.FC<WorktreeMenuProps>;
export {};
