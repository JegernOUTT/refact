import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeSurfaceKey } from "../surfaceKey";
import {
  bindSurfaceToChat,
  openTab,
  selectFocusedWorkspaceChatId,
  setDockOpen,
  type WorkspaceState,
} from "../workspaceSlice";
import type { ChangesEntryKind } from "./changesTreeModel";

export type GitDiffTarget = {
  root: string;
  path: string;
  old_path: string | null;
  kind: ChangesEntryKind;
};

export type GitRootViewState = {
  excludedChanges: string[];
  includedUnversioned: string[];
  groupByDirectory: boolean;
  collapsed: string[];
  selectedId: string | null;
};

export type GitPanelContextState = {
  activeRoot: string;
  selectedFile: GitDiffTarget | null;
  roots?: Record<string, GitRootViewState | undefined>;
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

export const DEFAULT_GIT_ROOT_VIEW: GitRootViewState = {
  excludedChanges: [],
  includedUnversioned: [],
  groupByDirectory: true,
  collapsed: [],
  selectedId: null,
};

const gitPanelContextKey = (chatId: string | null): string =>
  chatId ? `chat:${chatId}` : "workspace";

type ActiveGitRootPayload = {
  chatId: string | null;
  root: string;
};

type GitDiffTargetPayload = {
  chatId: string | null;
  selection: GitDiffTarget | null;
};

type RootViewPayload = {
  chatId: string | null;
  root: string;
  patch: Partial<GitRootViewState>;
};

const ensureContext = (
  state: GitPanelState,
  chatId: string | null,
): GitPanelContextState => {
  const key = gitPanelContextKey(chatId);
  const existing = state.contexts[key];
  if (existing) return existing;
  const created: GitPanelContextState = {
    activeRoot: "",
    selectedFile: null,
  };
  state.contexts[key] = created;
  return created;
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
        roots: context?.roots,
      };
    },
    selectGitDiffTarget: (
      state,
      action: PayloadAction<GitDiffTargetPayload>,
    ) => {
      const key = gitPanelContextKey(action.payload.chatId);
      const context = state.contexts[key];
      if (!action.payload.selection && !context) return;
      state.contexts[key] = {
        activeRoot: action.payload.selection?.root ?? context?.activeRoot ?? "",
        selectedFile: action.payload.selection,
        roots: context?.roots,
      };
    },
    updateGitRootView: (state, action: PayloadAction<RootViewPayload>) => {
      const context = ensureContext(state, action.payload.chatId);
      const roots = context.roots ?? {};
      roots[action.payload.root] = {
        ...DEFAULT_GIT_ROOT_VIEW,
        ...roots[action.payload.root],
        ...action.payload.patch,
      };
      context.roots = roots;
    },
  },
});

export const { selectGitDiffTarget, setActiveGitRoot, updateGitRootView } =
  gitPanelSlice.actions;

type GitPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof bindSurfaceToChat>
    | ReturnType<typeof selectGitDiffTarget>
    | ReturnType<typeof setDockOpen>,
) => void;

type GitPanelThunkState = {
  workspace: WorkspaceState;
};

export const openGitDiff =
  (target: GitDiffTarget) =>
  (dispatch: GitPanelDispatch, getState: () => GitPanelThunkState) => {
    const chatId = selectFocusedWorkspaceChatId(getState());
    const surfaceKey = makeSurfaceKey("git", "main");
    dispatch(selectGitDiffTarget({ chatId, selection: target }));
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

export const selectGitDiffTargetForChat = (
  state: GitPanelRootState,
  chatId: string | null,
) => selectGitPanelContext(state, chatId).selectedFile;

export const selectGitRootView = (
  state: GitPanelRootState,
  chatId: string | null,
  root: string,
): GitRootViewState =>
  selectGitPanelContext(state, chatId).roots?.[root] ?? DEFAULT_GIT_ROOT_VIEW;

export default gitPanelSlice.reducer;
