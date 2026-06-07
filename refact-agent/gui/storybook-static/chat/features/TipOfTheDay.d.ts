import { Selector } from 'reselect';
import { Slice, CaseReducer, ReducerType, ActionCreatorWithPayload } from '@reduxjs/toolkit';
import type { Config } from "../features/Config/configSlice";
type TipHost = "all" | "vscode";
export declare const tips: [TipHost, string][];
export type TipOfTheDayState = {
    current: number;
    tip: string;
};
export declare const tipOfTheDaySlice: Slice<TipOfTheDayState, {
    nextTip: CaseReducer<TipOfTheDayState, {
        payload: {
            host: Config["host"];
            completeManual?: string;
        };
        type: string;
    }> & {
        _reducerDefinitionType: ReducerType.reducer;
    };
}, "tipOfTheDay", "tipOfTheDay", {
    currentTipOfTheDay: (state: TipOfTheDayState) => string;
}>;
export declare const nextTip: ActionCreatorWithPayload<{
    host: Config["host"];
    completeManual?: string;
}, "tipOfTheDay/nextTip">;
export declare const currentTipOfTheDay: Selector<{
    tipOfTheDay: TipOfTheDayState;
}, string, []> & {
    unwrapped: (state: TipOfTheDayState) => string;
};
export {};
