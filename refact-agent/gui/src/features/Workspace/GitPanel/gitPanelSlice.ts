import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeSurfaceKey } from "../surfaceKey";
import {
  bindSurfaceToChat,
  openTab,
  selectFocusedWorkspaceChatId,
  setDockOpen,
  type WorkspaceState,
} from "../workspaceSlice";
import type { SelectedGitFile } from "./StatusList";

export type GitFileSelection = SelectedGitFile & {
  root: string;
};

export type GitPanelContextState = {
  activeRoot: string;
  selectedFile: GitFileSelection | null;
};

export type GitPanelState = {
  contexts: Record<string, GitPanelContextState | undefined>;
};

const initialState: GitPanelState = {
  contexts: {},
};

const EMPTY_GIT_PANEL_CONTEXT: GitPanelContextState = {
  activeRoot: "",
  selectedFile: null,
};

const gitPanelContextKey = (chatId: string | null): string =>
  chatId ? `chat:${chatId}` : "workspace";

type ActiveGitRootPayload = {
  chatId: string | null;
  root: string;
};

type GitFileSelectionPayload = {
  chatId: string | null;
  selection: GitFileSelection | null;
};

export const gitPanelSlice = createSlice({
  name: "gitPanel",
  reducerPath: "gitPanel",
  initialState,
  reducers: {
    setActiveGitRoot: (state, action: PayloadAction<ActiveGitRootPayload>) => {
      const key = gitPanelContextKey(action.payload.chatId);
      const context = state.contexts[key];
      if ((context?.activeRoot ?? "") === action.payload.root) return;
      state.contexts[key] = {
        activeRoot: action.payload.root,
        selectedFile: null,
      };
    },
    selectGitFile: (state, action: PayloadAction<GitFileSelectionPayload>) => {
      const key = gitPanelContextKey(action.payload.chatId);
      const context = state.contexts[key];
      if (!action.payload.selection && !context) return;
      state.contexts[key] = {
        activeRoot: action.payload.selection?.root ?? context?.activeRoot ?? "",
        selectedFile: action.payload.selection,
      };
    },
  },
});

export const { selectGitFile, setActiveGitRoot } = gitPanelSlice.actions;

type GitPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof bindSurfaceToChat>
    | ReturnType<typeof selectGitFile>
    | ReturnType<typeof setDockOpen>,
) => void;

type GitPanelThunkState = {
  workspace: WorkspaceState;
};

export const openGitFile =
  (selection: GitFileSelection) =>
  (dispatch: GitPanelDispatch, getState: () => GitPanelThunkState) => {
    const chatId = selectFocusedWorkspaceChatId(getState());
    const surfaceKey = makeSurfaceKey("git", "main");
    dispatch(selectGitFile({ chatId, selection }));
    dispatch(openTab(surfaceKey));
    if (chatId) dispatch(bindSurfaceToChat({ surfaceKey, chatId }));
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

const selectGitPanelContext = (
  state: GitPanelRootState,
  chatId: string | null,
): GitPanelContextState =>
  state.gitPanel.contexts[gitPanelContextKey(chatId)] ??
  EMPTY_GIT_PANEL_CONTEXT;

export const selectActiveGitRoot = (
  state: GitPanelRootState,
  chatId: string | null,
) => selectGitPanelContext(state, chatId).activeRoot;

export const selectSelectedGitFile = (
  state: GitPanelRootState,
  chatId: string | null,
) => selectGitPanelContext(state, chatId).selectedFile;

export default gitPanelSlice.reducer;
