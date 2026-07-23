import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeSurfaceKey } from "../surfaceKey";
import { openTab, setDockOpen } from "../workspaceSlice";
import type { SelectedGitFile } from "./StatusList";

export type GitFileSelection = SelectedGitFile & {
  root: string;
};

export type GitPanelState = {
  activeRoot: string;
  selectedFile: GitFileSelection | null;
};

const initialState: GitPanelState = {
  activeRoot: "",
  selectedFile: null,
};

export const gitPanelSlice = createSlice({
  name: "gitPanel",
  reducerPath: "gitPanel",
  initialState,
  reducers: {
    setActiveGitRoot: (state, action: PayloadAction<string>) => {
      if (state.activeRoot === action.payload) return;
      state.activeRoot = action.payload;
      state.selectedFile = null;
    },
    selectGitFile: (state, action: PayloadAction<GitFileSelection | null>) => {
      state.selectedFile = action.payload;
      if (action.payload) state.activeRoot = action.payload.root;
    },
  },
});

export const { selectGitFile, setActiveGitRoot } = gitPanelSlice.actions;

type GitPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof selectGitFile>
    | ReturnType<typeof setDockOpen>,
) => void;

export const openGitFile =
  (selection: GitFileSelection) => (dispatch: GitPanelDispatch) => {
    dispatch(selectGitFile(selection));
    dispatch(openTab(makeSurfaceKey("git", "main")));
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(max-width: 767px)").matches
    ) {
      dispatch(setDockOpen(false));
    }
  };

type GitPanelRootState = {
  gitPanel: GitPanelState;
};

export const selectActiveGitRoot = (state: GitPanelRootState) =>
  state.gitPanel.activeRoot;

export const selectSelectedGitFile = (state: GitPanelRootState) =>
  state.gitPanel.selectedFile;

export default gitPanelSlice.reducer;
