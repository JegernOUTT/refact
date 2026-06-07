export type Checkpoint = {
    workspace_folder: string;
    commit_hash: string;
};
export type FileChangedStatus = "ADDED" | "MODIFIED" | "DELETED";
export type FileChanged = {
    absolute_path: string;
    relative_path: string;
    status: FileChangedStatus;
};
export type RevertedCheckpointData = {
    workspace_folder: string;
    files_changed: FileChanged[];
};
export type PreviewCheckpointsPayload = {
    checkpoints: Checkpoint[];
    chat_id: string;
    chat_mode?: string;
};
export type RestoreCheckpointsPayload = {
    checkpoints: Checkpoint[];
    chat_id: string;
    chat_mode?: string;
};
export type PreviewCheckpointsResponse = {
    reverted_to: string;
    checkpoints_for_undo: Checkpoint[];
    reverted_changes: RevertedCheckpointData[];
    error_log: string[];
};
export type RestoreCheckpointsResponse = {
    success: boolean;
    error_log: string[];
};
export declare function isRestoreCheckpointsResponse(json: unknown): json is RestoreCheckpointsResponse;
export declare function isPreviewCheckpointsResponse(json: unknown): json is PreviewCheckpointsResponse;
