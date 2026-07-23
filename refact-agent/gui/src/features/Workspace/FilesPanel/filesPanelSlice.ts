import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeSurfaceKey } from "../surfaceKey";
import { openTab, setDockOpen } from "../workspaceSlice";

export type FileViewerTarget = {
  path: string;
  line?: number;
};

export type FilesPanelState = {
  expandedDirectories: string[];
  selectedPath: string | null;
  viewerTarget: FileViewerTarget | null;
  viewerTargets: Record<string, FileViewerTarget | undefined>;
};

const initialState: FilesPanelState = {
  expandedDirectories: [],
  selectedPath: null,
  viewerTarget: null,
  viewerTargets: {},
};

export const filesPanelSlice = createSlice({
  name: "filesPanel",
  reducerPath: "filesPanel",
  initialState,
  reducers: {
    toggleDirectory: (state, action: PayloadAction<string>) => {
      const index = state.expandedDirectories.indexOf(action.payload);
      if (index === -1) state.expandedDirectories.push(action.payload);
      else state.expandedDirectories.splice(index, 1);
      state.selectedPath = action.payload;
    },
    expandDirectory: (state, action: PayloadAction<string>) => {
      if (!state.expandedDirectories.includes(action.payload)) {
        state.expandedDirectories.push(action.payload);
      }
    },
    collapseDirectory: (state, action: PayloadAction<string>) => {
      state.expandedDirectories = state.expandedDirectories.filter(
        (path) => path !== action.payload,
      );
    },
    selectTreePath: (state, action: PayloadAction<string>) => {
      state.selectedPath = action.payload;
    },
    setViewerTarget: (
      state,
      action: PayloadAction<FileViewerTarget | null>,
    ) => {
      state.viewerTarget = action.payload;
      state.selectedPath = action.payload?.path ?? state.selectedPath;
      if (action.payload) {
        state.viewerTargets[action.payload.path] = action.payload;
      }
    },
  },
});

export const {
  collapseDirectory,
  expandDirectory,
  selectTreePath,
  setViewerTarget,
  toggleDirectory,
} = filesPanelSlice.actions;

type FilesPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof expandDirectory>
    | ReturnType<typeof setViewerTarget>
    | ReturnType<typeof setDockOpen>,
) => void;

const parentDirectories = (path: string): string[] => {
  const normalized = path.replace(/\\/g, "/");
  const parent = normalized.slice(0, normalized.lastIndexOf("/"));
  const rootPrefix = parent.startsWith("/") ? "/" : "";
  const segments = parent.split("/").filter(Boolean);
  return segments.map(
    (_, index) => rootPrefix + segments.slice(0, index + 1).join("/"),
  );
};

export const openFileInFilesPanel =
  (target: FileViewerTarget) => (dispatch: FilesPanelDispatch) => {
    dispatch(openTab(makeSurfaceKey("file", target.path)));
    for (const directory of parentDirectories(target.path)) {
      dispatch(expandDirectory(directory));
    }
    dispatch(setViewerTarget(target));
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(max-width: 767px)").matches
    ) {
      dispatch(setDockOpen(false));
    }
  };

type FilesPanelRootState = {
  filesPanel: FilesPanelState;
};

export const selectExpandedDirectories = (state: FilesPanelRootState) =>
  state.filesPanel.expandedDirectories;

export const selectFilesPanelSelectedPath = (state: FilesPanelRootState) =>
  state.filesPanel.selectedPath;

export const selectFileViewerTarget = (state: FilesPanelRootState) =>
  state.filesPanel.viewerTarget;

export const selectFileViewerTargetByPath = (
  state: FilesPanelRootState,
  path: string,
): FileViewerTarget | undefined => state.filesPanel.viewerTargets[path];
