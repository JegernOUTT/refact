import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, type PayloadAction } from '@reduxjs/toolkit';
import type { UserErrorCategory, UserErrorInfo } from "../../services/refact/types";
export type ErrorPayload = string | {
    message: string;
    error_info?: UserErrorInfo;
};
export type ErrorSliceState = {
    message: string | null;
    isAuthError?: boolean;
    category?: UserErrorCategory;
    error_info?: UserErrorInfo;
};
export declare const errorSlice: Slice<ErrorSliceState, {
    setError: (state: WritableDraft<ErrorSliceState>, action: PayloadAction<ErrorPayload>) => void;
    setIsAuthError: (state: WritableDraft<ErrorSliceState>, action: PayloadAction<boolean>) => void;
    clearError: (state: WritableDraft<ErrorSliceState>, _action: PayloadAction) => void;
}, "error", "error", {
    getErrorMessage: (state: ErrorSliceState) => string | null;
    getIsAuthError: (state: ErrorSliceState) => boolean | undefined;
    getErrorCategory: (state: ErrorSliceState) => UserErrorCategory | undefined;
    getErrorInfo: (state: ErrorSliceState) => UserErrorInfo | undefined;
}>;
export declare const setError: ActionCreatorWithPayload<ErrorPayload, "error/setError">, setIsAuthError: ActionCreatorWithPayload<boolean, "error/setIsAuthError">, clearError: ActionCreatorWithoutPayload<"error/clearError">;
export declare const getErrorMessage: Selector<{
    error: ErrorSliceState;
}, string | null, []> & {
    unwrapped: (state: ErrorSliceState) => string | null;
}, getIsAuthError: Selector<{
    error: ErrorSliceState;
}, boolean | undefined, []> & {
    unwrapped: (state: ErrorSliceState) => boolean | undefined;
}, getErrorCategory: Selector<{
    error: ErrorSliceState;
}, UserErrorCategory | undefined, []> & {
    unwrapped: (state: ErrorSliceState) => UserErrorCategory | undefined;
}, getErrorInfo: Selector<{
    error: ErrorSliceState;
}, UserErrorInfo | undefined, []> & {
    unwrapped: (state: ErrorSliceState) => UserErrorInfo | undefined;
};
