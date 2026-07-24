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

const normalizePath = (path: string): string => {
  const normalized = path.replace(/\\/g, "/");
  if (/^\/+$/u.test(normalized)) return "/";
  if (/^[A-Za-z]:\/+$/u.test(normalized)) {
    return `${normalized.slice(0, 2)}/`;
  }
  return normalized.replace(/\/+$/u, "");
};

export const isPathWithinWorkspaceRoots = (
  path: string,
  workspaceRoots: string[],
): boolean => {
  const normalizedPath = normalizePath(path);
  return workspaceRoots.some((workspaceRoot) => {
    const normalizedRoot = normalizePath(workspaceRoot);
    if (!normalizedRoot) return false;
    if (normalizedPath === normalizedRoot) return true;
    if (normalizedRoot === "/") return normalizedPath.startsWith("/");
    if (normalizedRoot.endsWith("/")) {
      return normalizedPath.startsWith(normalizedRoot);
    }
    return normalizedPath.startsWith(`${normalizedRoot}/`);
  });
};

const parentDirectories = (path: string): string[] => {
  const normalized = normalizePath(path);
  const lastSeparator = normalized.lastIndexOf("/");
  const parent = lastSeparator === 0 ? "/" : normalized.slice(0, lastSeparator);
  if (parent === "/") return [parent];
  const rootPrefix = parent.startsWith("//")
    ? "//"
    : parent.startsWith("/")
      ? "/"
      : "";
  const segments = parent.split("/").filter(Boolean);
  return segments.map((_, index) => {
    const directory = rootPrefix + segments.slice(0, index + 1).join("/");
    if (index === 0 && /^[A-Za-z]:$/u.test(directory)) return `${directory}/`;
    return directory;
  });
};

type FilesPanelThunkState = {
  current_project: {
    workspaceRoots?: string[];
  };
};

export const openFileInFilesPanel =
  (target: FileViewerTarget) =>
  (dispatch: FilesPanelDispatch, getState: () => FilesPanelThunkState) => {
    dispatch(openTab(makeSurfaceKey("file", target.path)));
    const workspaceRoots = getState().current_project.workspaceRoots ?? [];
    for (const directory of parentDirectories(target.path)) {
      if (isPathWithinWorkspaceRoots(directory, workspaceRoots)) {
        dispatch(expandDirectory(directory));
      }
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
