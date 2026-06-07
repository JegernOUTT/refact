import React from "react";
import type { WorktreeMeta, WorktreeRecordView } from "../../services/refact";
type WorktreeStatusBadgeProps = {
    worktree?: WorktreeMeta | null;
    record?: WorktreeRecordView | null;
    additions?: number | null;
    deletions?: number | null;
};
export declare const WorktreeStatusBadge: React.FC<WorktreeStatusBadgeProps>;
export {};
