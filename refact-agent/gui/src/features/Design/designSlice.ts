import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { RootState } from "../../app/store";
import type { DesignElementSelection, DesignTheme } from "./surfaceContract";

export type DesignSourceKind = "live" | "artifact" | "reference";
export type DesignViewportPreset = "375" | "768" | "1280" | "1440" | "custom";
export type DesignLiveStatus =
  | "idle"
  | "probing"
  | "interactive"
  | "basic"
  | "blocked";
export type ReferenceCompareMode = "side-by-side" | "overlay";

export type DesignSurfaceState = {
  source: DesignSourceKind;
  liveUrl: string;
  liveStatus: DesignLiveStatus;
  fallbackReason: string | null;
  artifactHtml: string;
  referenceDataUrl: string | null;
  compareMode: ReferenceCompareMode;
  overlayOpacity: number;
  viewportPreset: DesignViewportPreset;
  customWidth: number;
  theme: DesignTheme;
  devicePixelRatio: number;
  zoom: number;
  pickerEnabled: boolean;
  refreshNonce: number;
  selection: DesignElementSelection | null;
};

export type DesignState = {
  surfaces: Record<string, DesignSurfaceState | undefined>;
};

export const makeDesignSurfaceState = (): DesignSurfaceState => ({
  source: "live",
  liveUrl: "",
  liveStatus: "idle",
  fallbackReason: null,
  artifactHtml: "",
  referenceDataUrl: null,
  compareMode: "side-by-side",
  overlayOpacity: 50,
  viewportPreset: "1280",
  customWidth: 1024,
  theme: "light",
  devicePixelRatio: 1,
  zoom: 100,
  pickerEnabled: false,
  refreshNonce: 0,
  selection: null,
});

const initialState: DesignState = { surfaces: {} };

type SurfacePatch = {
  surfaceId: string;
  patch: Partial<DesignSurfaceState>;
};

export const designSlice = createSlice({
  name: "design",
  reducerPath: "design",
  initialState,
  reducers: {
    ensureDesignSurface(state, action: PayloadAction<string>) {
      state.surfaces[action.payload] ??= makeDesignSurfaceState();
    },
    updateDesignSurface(state, action: PayloadAction<SurfacePatch>) {
      const current =
        state.surfaces[action.payload.surfaceId] ?? makeDesignSurfaceState();
      state.surfaces[action.payload.surfaceId] = {
        ...current,
        ...action.payload.patch,
      };
    },
    refreshDesignSurface(state, action: PayloadAction<string>) {
      const current =
        state.surfaces[action.payload] ?? makeDesignSurfaceState();
      current.refreshNonce += 1;
      current.liveStatus = current.liveUrl ? "probing" : "idle";
      current.fallbackReason = null;
      current.selection = null;
      state.surfaces[action.payload] = current;
    },
  },
});

export const {
  ensureDesignSurface,
  refreshDesignSurface,
  updateDesignSurface,
} = designSlice.actions;

export const selectDesignSurface = (
  state: RootState,
  surfaceId: string,
): DesignSurfaceState | undefined => state.design.surfaces[surfaceId];
