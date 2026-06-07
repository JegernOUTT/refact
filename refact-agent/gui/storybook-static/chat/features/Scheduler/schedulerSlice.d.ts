import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, PayloadAction } from '@reduxjs/toolkit';
export type SchedulerScope = "all" | "session" | "durable";
export type SchedulerState = {
    scope: SchedulerScope;
    lastCronFireAt: number | null;
};
export declare const schedulerSlice: Slice<SchedulerState, {
    setSchedulerScope: (state: WritableDraft<SchedulerState>, action: PayloadAction<SchedulerScope>) => void;
    cronFireReceived: (state: WritableDraft<SchedulerState>, action: PayloadAction<number>) => void;
}, "scheduler", "scheduler", {
    selectSchedulerScope: (state: SchedulerState) => SchedulerScope;
    selectLastCronFireAt: (state: SchedulerState) => number | null;
}>;
export declare const setSchedulerScope: ActionCreatorWithPayload<SchedulerScope, "scheduler/setSchedulerScope">, cronFireReceived: ActionCreatorWithPayload<number, "scheduler/cronFireReceived">;
export declare const selectSchedulerScope: Selector<{
    scheduler: SchedulerState;
}, SchedulerScope, []> & {
    unwrapped: (state: SchedulerState) => SchedulerScope;
}, selectLastCronFireAt: Selector<{
    scheduler: SchedulerState;
}, number | null, []> & {
    unwrapped: (state: SchedulerState) => number | null;
};
