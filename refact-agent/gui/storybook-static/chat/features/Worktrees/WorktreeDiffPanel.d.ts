import React from "react";
import { type WorktreeMeta, type WorktreeRecordView } from "../../services/refact";
type WorktreeDiffPanelProps = {
    open: boolean;
    worktreeId?: string | null;
    worktree?: WorktreeMeta | null;
    record?: WorktreeRecordView | null;
    sourceWorkspaceRoot?: string;
    onOpenChange: (open: boolean) => void;
    closeOnNonInteractiveContentClick?: boolean;
};
export declare const WorktreeDiffPanel: React.FC<WorktreeDiffPanelProps>;
export {};
