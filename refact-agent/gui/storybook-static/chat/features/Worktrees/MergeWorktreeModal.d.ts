import React from "react";
import { type MergeWorktreeResponse, type WorktreeMeta, type WorktreeRecordView } from "../../services/refact";
type MergeWorktreeModalProps = {
    open: boolean;
    worktreeId?: string | null;
    worktree?: WorktreeMeta | null;
    record?: WorktreeRecordView | null;
    taskId?: string;
    defaultTargetBranch?: string | null;
    onOpenChange: (open: boolean) => void;
    onMerged?: (response: MergeWorktreeResponse) => void;
    onAskRefact?: (files: string[], response: MergeWorktreeResponse) => void | Promise<void>;
    onOpenWorktree?: () => void | Promise<void>;
    closeOnNonInteractiveContentClick?: boolean;
};
export declare const MergeWorktreeModal: React.FC<MergeWorktreeModalProps>;
export {};
