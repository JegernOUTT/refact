import { Selector } from 'reselect';
import { KnowledgeMemoRecord } from '../../services/refact';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, PayloadAction } from '@reduxjs/toolkit';
import type { MemoRecord, VecDbStatus } from "../../services/refact/types";
export type KnowledgeState = {
    loaded: boolean;
    memories: Record<string, MemoRecord>;
    status: null | VecDbStatus;
};
export declare const knowledgeSlice: Slice<KnowledgeState, {
    setVecDbStatus: (state: WritableDraft<KnowledgeState>, action: PayloadAction<VecDbStatus>) => void;
    setMemory: (state: WritableDraft<KnowledgeState>, action: PayloadAction<MemoRecord>) => void;
    deleteMemory: (state: WritableDraft<KnowledgeState>, action: PayloadAction<string>) => void;
    clearMemory: (state: WritableDraft<KnowledgeState>) => void;
}, "knowledge", "knowledge", {
    selectVecDbStatus: (state: KnowledgeState) => VecDbStatus | null;
    selectMemories: (state: KnowledgeState) => Record<string, KnowledgeMemoRecord>;
    selectKnowledgeIsLoaded: (state: KnowledgeState) => boolean;
}>;
export declare const setVecDbStatus: ActionCreatorWithPayload<VecDbStatus, "knowledge/setVecDbStatus">, setMemory: ActionCreatorWithPayload<KnowledgeMemoRecord, "knowledge/setMemory">, deleteMemory: ActionCreatorWithPayload<string, "knowledge/deleteMemory">, clearMemory: ActionCreatorWithoutPayload<"knowledge/clearMemory">;
export declare const selectVecDbStatus: Selector<{
    knowledge: KnowledgeState;
}, VecDbStatus | null, []> & {
    unwrapped: (state: KnowledgeState) => VecDbStatus | null;
}, selectMemories: Selector<{
    knowledge: KnowledgeState;
}, Record<string, KnowledgeMemoRecord>, []> & {
    unwrapped: (state: KnowledgeState) => Record<string, KnowledgeMemoRecord>;
}, selectKnowledgeIsLoaded: Selector<{
    knowledge: KnowledgeState;
}, boolean, []> & {
    unwrapped: (state: KnowledgeState) => boolean;
};
