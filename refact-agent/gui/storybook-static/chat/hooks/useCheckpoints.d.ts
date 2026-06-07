import type { RestoreMode } from "../features/Checkpoints/Checkpoints";
import { RevertedCheckpointData, Checkpoint, FileChanged } from '../features/Checkpoints/types';
export declare const useCheckpoints: () => {
    shouldCheckpointsPopupBeShown: boolean;
    handleUndo: () => void;
    handlePreview: (checkpoints: Checkpoint[] | null, messageIndex: number) => Promise<void>;
    handleFix: (restoreMode?: RestoreMode) => Promise<void>;
    isRestoring: boolean;
    isPreviewing: boolean;
    reverted_changes: RevertedCheckpointData[];
    reverted_to: string;
    wereFilesChanged: boolean;
    allChangedFiles: (FileChanged & {
        workspace_folder: string;
    })[];
    errorLog: string[];
};
