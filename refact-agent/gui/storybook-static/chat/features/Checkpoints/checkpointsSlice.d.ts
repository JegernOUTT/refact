import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, PayloadAction } from '@reduxjs/toolkit';
import { Checkpoint, PreviewCheckpointsResponse } from "./types";
export type CheckpointsMeta = {
    latestCheckpointResult: PreviewCheckpointsResponse & {
        current_checkpoints: Checkpoint[];
        chat_id: string;
        chat_mode?: string;
    };
    isVisible: boolean;
    isUndoing: boolean;
    restoringUserMessageIndex: number | null;
    shouldNewChatBeStarted: boolean;
};
export declare const checkpointsSlice: Slice<CheckpointsMeta, {
    setLatestCheckpointResult: (state: WritableDraft<CheckpointsMeta>, action: PayloadAction<PreviewCheckpointsResponse & {
        messageIndex: number;
        current_checkpoints: Checkpoint[];
        chat_id: string;
        chat_mode?: string;
    }>) => void;
    setIsCheckpointsPopupIsVisible: (state: WritableDraft<CheckpointsMeta>, action: PayloadAction<boolean>) => void;
    setIsUndoingCheckpoints: (state: WritableDraft<CheckpointsMeta>, action: PayloadAction<boolean>) => void;
    setShouldNewChatBeStarted: (state: WritableDraft<CheckpointsMeta>, action: PayloadAction<boolean>) => void;
    setCheckpointsErrorLog: (state: WritableDraft<CheckpointsMeta>, action: PayloadAction<string[]>) => void;
    clearCheckpointsErrorLog: (state: WritableDraft<CheckpointsMeta>) => void;
}, "checkpoints", "checkpoints", {
    selectLatestCheckpointResult: (state: CheckpointsMeta) => PreviewCheckpointsResponse & {
        current_checkpoints: Checkpoint[];
        chat_id: string;
        chat_mode?: string;
    };
    selectIsCheckpointsPopupIsVisible: (state: CheckpointsMeta) => boolean;
    selectIsUndoingCheckpoints: (state: CheckpointsMeta) => boolean;
    selectShouldNewChatBeStarted: (state: CheckpointsMeta) => boolean;
    selectCheckpointsMessageIndex: (state: CheckpointsMeta) => number | null;
}>;
export declare const setLatestCheckpointResult: ActionCreatorWithPayload<PreviewCheckpointsResponse & {
    messageIndex: number;
    current_checkpoints: Checkpoint[];
    chat_id: string;
    chat_mode?: string;
}, "checkpoints/setLatestCheckpointResult">, setIsCheckpointsPopupIsVisible: ActionCreatorWithPayload<boolean, "checkpoints/setIsCheckpointsPopupIsVisible">, setIsUndoingCheckpoints: ActionCreatorWithPayload<boolean, "checkpoints/setIsUndoingCheckpoints">, setShouldNewChatBeStarted: ActionCreatorWithPayload<boolean, "checkpoints/setShouldNewChatBeStarted">, setCheckpointsErrorLog: ActionCreatorWithPayload<string[], "checkpoints/setCheckpointsErrorLog">, clearCheckpointsErrorLog: ActionCreatorWithoutPayload<"checkpoints/clearCheckpointsErrorLog">;
export declare const selectLatestCheckpointResult: Selector<{
    checkpoints: CheckpointsMeta;
}, PreviewCheckpointsResponse & {
    current_checkpoints: Checkpoint[];
    chat_id: string;
    chat_mode?: string;
}, []> & {
    unwrapped: (state: CheckpointsMeta) => PreviewCheckpointsResponse & {
        current_checkpoints: Checkpoint[];
        chat_id: string;
        chat_mode?: string;
    };
}, selectIsCheckpointsPopupIsVisible: Selector<{
    checkpoints: CheckpointsMeta;
}, boolean, []> & {
    unwrapped: (state: CheckpointsMeta) => boolean;
}, selectIsUndoingCheckpoints: Selector<{
    checkpoints: CheckpointsMeta;
}, boolean, []> & {
    unwrapped: (state: CheckpointsMeta) => boolean;
}, selectShouldNewChatBeStarted: Selector<{
    checkpoints: CheckpointsMeta;
}, boolean, []> & {
    unwrapped: (state: CheckpointsMeta) => boolean;
}, selectCheckpointsMessageIndex: Selector<{
    checkpoints: CheckpointsMeta;
}, number | null, []> & {
    unwrapped: (state: CheckpointsMeta) => number | null;
};
