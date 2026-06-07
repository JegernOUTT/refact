import type { BoardCard } from "../../services/refact/tasks";
import type { WorktreeMeta, WorktreeRecordView } from "../../services/refact";
export type CardWorktreeTarget = {
    id: string;
    label: string;
    record?: WorktreeRecordView;
    meta?: WorktreeMeta | null;
    legacy: boolean;
    stale: boolean;
    referenceCount?: number;
};
export declare function worktreeLabel(card: BoardCard, record?: WorktreeRecordView, meta?: WorktreeMeta | null): string | null;
export declare function isActionableWorktree(worktree: CardWorktreeTarget): boolean;
export declare function resolveCardWorktree(taskId: string, card: BoardCard, records: WorktreeRecordView[], threadWorktree?: WorktreeMeta | null): CardWorktreeTarget | null;
