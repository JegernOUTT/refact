import type { MergeWorktreeResponse, WorktreeMeta, WorktreeRecordView } from "../../services/refact";
type ConflictPromptArgs = {
    worktree?: WorktreeMeta | null;
    record?: WorktreeRecordView | null;
    response?: MergeWorktreeResponse | null;
    files: string[];
    taskId?: string;
    cardId?: string;
};
export declare function mergeConflictFiles(response: MergeWorktreeResponse): string[];
export declare function buildWorktreeConflictPrompt({ worktree, record, response, files, taskId, cardId, }: ConflictPromptArgs): string;
export {};
