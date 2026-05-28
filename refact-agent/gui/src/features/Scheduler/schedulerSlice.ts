import { createSlice, PayloadAction } from "@reduxjs/toolkit";

export type SchedulerScope = "all" | "session" | "durable";

export type SchedulerState = {
  scope: SchedulerScope;
  lastCronFireAt: number | null;
};

const initialState: SchedulerState = {
  scope: "all",
  lastCronFireAt: null,
};

export const schedulerSlice = createSlice({
  name: "scheduler",
  initialState,
  reducers: {
    setSchedulerScope: (state, action: PayloadAction<SchedulerScope>) => {
      state.scope = action.payload;
    },
    cronFireReceived: (state, action: PayloadAction<number>) => {
      state.lastCronFireAt = action.payload;
    },
  },
  selectors: {
    selectSchedulerScope: (state) => state.scope,
    selectLastCronFireAt: (state) => state.lastCronFireAt,
  },
});

export const { setSchedulerScope, cronFireReceived } = schedulerSlice.actions;
export const { selectSchedulerScope, selectLastCronFireAt } =
  schedulerSlice.selectors;
