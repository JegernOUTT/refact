import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

export type PrivacyState = {
  selectedZoneName: string | null;
};

const initialState: PrivacyState = {
  selectedZoneName: null,
};

export const privacySlice = createSlice({
  name: "privacy",
  initialState,
  reducers: {
    setSelectedZone: (state, action: PayloadAction<string | null>) => {
      state.selectedZoneName = action.payload;
    },
    clearSelectedZone: (state) => {
      state.selectedZoneName = null;
    },
  },
  selectors: {
    selectSelectedZoneName: (state) => state.selectedZoneName,
  },
});

export const { setSelectedZone, clearSelectedZone } = privacySlice.actions;
export const { selectSelectedZoneName } = privacySlice.selectors;
