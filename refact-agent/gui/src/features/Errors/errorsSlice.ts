import { createSlice, type PayloadAction } from "@reduxjs/toolkit";
import type {
  UserErrorCategory,
  UserErrorInfo,
} from "../../services/refact/types";

export type ErrorSource = "caps_providers";

export type ErrorDetails = {
  message: string;
  error_info?: UserErrorInfo;
  source?: ErrorSource;
};

export type ErrorPayload = string | ErrorDetails;

export type ErrorSlotState = {
  message: string | null;
  category?: UserErrorCategory;
  error_info?: UserErrorInfo;
};

export type ErrorSliceState = ErrorSlotState & {
  isAuthError?: boolean;
  capsProviders: ErrorSlotState;
};

const initialState: ErrorSliceState = {
  message: null,
  capsProviders: { message: null },
};

function clearSlot(slot: ErrorSlotState) {
  slot.message = null;
  slot.category = undefined;
  slot.error_info = undefined;
}

export const errorSlice = createSlice({
  name: "error",
  initialState,
  reducers: {
    setError: (state, action: PayloadAction<ErrorPayload>) => {
      const payload: ErrorDetails =
        typeof action.payload === "string"
          ? { message: action.payload }
          : action.payload;
      const slot: ErrorSlotState =
        payload.source === "caps_providers" ? state.capsProviders : state;

      if (slot.message) return;

      slot.message = payload.message;
      slot.category = payload.error_info?.category;
      slot.error_info = payload.error_info;
    },
    setIsAuthError: (state, action: PayloadAction<boolean>) => {
      state.isAuthError = action.payload;
    },
    clearError: (state, _action: PayloadAction) => {
      clearSlot(state);
      state.isAuthError = undefined;
    },
    clearCapsProvidersError: (state, _action: PayloadAction) => {
      clearSlot(state.capsProviders);
      state.isAuthError = undefined;
    },
  },
  selectors: {
    getErrorMessage: (state) => state.message ?? state.capsProviders.message,
    getIsAuthError: (state) => state.isAuthError,
    getErrorCategory: (state) => state.category ?? state.capsProviders.category,
    getErrorInfo: (state) => state.error_info ?? state.capsProviders.error_info,
    getCapsProvidersErrorMessage: (state) => state.capsProviders.message,
    getRouteErrorMessage: (state) => state.message,
  },
});

export const { setError, setIsAuthError, clearError, clearCapsProvidersError } =
  errorSlice.actions;
export const {
  getErrorMessage,
  getIsAuthError,
  getErrorCategory,
  getErrorInfo,
  getCapsProvidersErrorMessage,
  getRouteErrorMessage,
} = errorSlice.selectors;
