import { createSlice, type PayloadAction } from "@reduxjs/toolkit";
import type {
  UserErrorCategory,
  UserErrorInfo,
} from "../../services/refact/types";

export type ErrorSource = "caps_providers";

export type ErrorPayload =
  | string
  | {
      message: string;
      error_info?: UserErrorInfo;
      source?: ErrorSource;
    };

export type ErrorSliceState = {
  message: string | null;
  isAuthError?: boolean;
  category?: UserErrorCategory;
  error_info?: UserErrorInfo;
  source?: ErrorSource;
};

const initialState: ErrorSliceState = { message: null };

function applyErrorPayload(state: ErrorSliceState, payload: ErrorPayload) {
  if (typeof payload === "string") {
    state.message = payload;
    state.category = undefined;
    state.error_info = undefined;
    state.source = undefined;
    return;
  }

  state.message = payload.message;
  state.category = payload.error_info?.category;
  state.error_info = payload.error_info;
  state.source = payload.source;
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
      state.source = undefined;
    },
  },
  selectors: {
    getErrorMessage: (state) => state.message,
    getIsAuthError: (state) => state.isAuthError,
    getErrorCategory: (state) => state.category,
    getErrorInfo: (state) => state.error_info,
    getErrorSource: (state) => state.source,
    getCapsProvidersErrorMessage: (state) =>
      state.source === "caps_providers" ? state.message : null,
    getRouteErrorMessage: (state) =>
      state.source === "caps_providers" ? null : state.message,
  },
});

export const { setError, setIsAuthError, clearError } = errorSlice.actions;
export const {
  getErrorMessage,
  getIsAuthError,
  getErrorCategory,
  getErrorInfo,
  getErrorSource,
  getCapsProvidersErrorMessage,
  getRouteErrorMessage,
} = errorSlice.selectors;
