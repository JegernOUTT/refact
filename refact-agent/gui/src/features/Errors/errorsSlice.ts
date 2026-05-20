import { createSlice, type PayloadAction } from "@reduxjs/toolkit";
import type { UserErrorCategory, UserErrorInfo } from "../../services/refact/types";

export type ErrorPayload =
  | string
  | {
      message: string;
      error_info?: UserErrorInfo;
    };

export type ErrorSliceState = {
  message: string | null;
  isAuthError?: boolean;
  category?: UserErrorCategory;
  error_info?: UserErrorInfo;
};

const initialState: ErrorSliceState = { message: null };

function applyErrorPayload(state: ErrorSliceState, payload: ErrorPayload) {
  if (typeof payload === "string") {
    state.message = payload;
    state.category = undefined;
    state.error_info = undefined;
    return;
  }

  state.message = payload.message;
  state.category = payload.error_info?.category;
  state.error_info = payload.error_info;
}

export const errorSlice = createSlice({
  name: "error",
  initialState,
  reducers: {
    setError: (state, action: PayloadAction<ErrorPayload>) => {
      if (state.message) return;
      applyErrorPayload(state, action.payload);
    },
    setIsAuthError: (state, action: PayloadAction<boolean>) => {
      state.isAuthError = action.payload;
    },
    clearError: (state, _action: PayloadAction) => {
      state.message = null;
      state.category = undefined;
      state.error_info = undefined;
      state.isAuthError = undefined;
    },
  },
  selectors: {
    getErrorMessage: (state) => state.message,
    getIsAuthError: (state) => state.isAuthError,
    getErrorCategory: (state) => state.category,
    getErrorInfo: (state) => state.error_info,
  },
});

export const { setError, setIsAuthError, clearError } = errorSlice.actions;
export const {
  getErrorMessage,
  getIsAuthError,
  getErrorCategory,
  getErrorInfo,
} = errorSlice.selectors;
