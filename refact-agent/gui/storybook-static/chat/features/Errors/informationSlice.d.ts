import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, type PayloadAction } from '@reduxjs/toolkit';
export type InformationSliceState = {
    message: string | null;
};
export declare const informationSlice: Slice<InformationSliceState, {
    setInformation: (state: WritableDraft<InformationSliceState>, action: PayloadAction<string>) => void;
    clearInformation: (state: WritableDraft<InformationSliceState>, _action: PayloadAction) => void;
}, "information", "information", {
    getInformationMessage: (state: InformationSliceState) => string | null;
}>;
export declare const setInformation: ActionCreatorWithPayload<string, "information/setInformation">, clearInformation: ActionCreatorWithoutPayload<"information/clearInformation">;
export declare const getInformationMessage: Selector<{
    information: InformationSliceState;
}, string | null, []> & {
    unwrapped: (state: InformationSliceState) => string | null;
};
