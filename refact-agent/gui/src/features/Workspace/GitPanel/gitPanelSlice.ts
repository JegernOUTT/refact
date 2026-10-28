import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeGitDiffSurfaceKey, type SurfaceKey } from "../surfaceKey";
import {
  bindSurfaceToChat,
  openTab,
  selectFocusedWorkspaceChatId,
  setDockOpen,
  type WorkspaceState,
} from "../workspaceSlice";
import type { ChangesEntryKind } from "./changesTreeModel";
import type { GitFileStatus } from "../../../services/refact/gitRead";
import type { GitDiffDescriptor } from "./diff/gitDiffDescriptor";

export type GitDiffTarget = {
  root: string;
  path: string;
  old_path: string | null;
  kind: ChangesEntryKind;
  /** Working-tree status of the change; drives which diff side is missing. */
  status?: GitFileStatus | null;
};

export type GitCommitDiffTarget = {
  root: string;
  path: string;
  old_path: string | null;
  status?: GitFileStatus | null;
  commit: { oid: string; short_oid: string };
  /** First parent oid; `null` for a root commit. */
  parent_oid: string | null;
};

export type GitDiffViewMode = "side-by-side" | "unified";

export type GitDiffViewerPrefs = {
  mode: GitDiffViewMode;
  collapseUnchanged: boolean;
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
  /** One-shot Git Log filter request; GitLog applies it once per nonce. */
  logRequest?: GitLogRequestState | null;
};

export type GitWindowTab = "log" | "worktrees";

export type GitLogRequestState = {
  root?: string;
  path?: string;
  ref?: string;
  nonce: number;
};

export type OpenGitLogOptions = {
  root?: string;
  path?: string;
  ref?: string;
};

export type GitLogLayout = {
  tab: GitWindowTab;
  branchesWidth: number;
  detailsWidth: number;
};

export type GitPanelState = {
  contexts: Record<string, GitPanelContextState | undefined>;
  /** Optional so older persisted/preloaded states stay valid. */
  diffViewer?: GitDiffViewerPrefs;
  logLayout?: GitLogLayout;
  /** Monotonic counter for log filter request nonces. */
  logRequestSeq?: number;
};

export const DEFAULT_GIT_LOG_LAYOUT: GitLogLayout = {
  tab: "log",
  branchesWidth: 240,
  detailsWidth: 380,
};

export const DEFAULT_GIT_DIFF_VIEWER_PREFS: GitDiffViewerPrefs = {
  mode: "side-by-side",
  collapseUnchanged: true,
};

const initialState: GitPanelState = {
  contexts: {},
  diffViewer: DEFAULT_GIT_DIFF_VIEWER_PREFS,
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

type GitLogRequestPayload = OpenGitLogOptions & { chatId: string | null };

type ConsumeGitLogRequestPayload = { chatId: string | null; nonce: number };

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
        logRequest: context?.logRequest,
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
        logRequest: context?.logRequest,
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
    setGitDiffViewerPrefs: (
      state,
      action: PayloadAction<Partial<GitDiffViewerPrefs>>,
    ) => {
      state.diffViewer = {
        ...DEFAULT_GIT_DIFF_VIEWER_PREFS,
        ...state.diffViewer,
        ...action.payload,
      };
    },
    setGitLogLayout: (state, action: PayloadAction<Partial<GitLogLayout>>) => {
      state.logLayout = {
        ...DEFAULT_GIT_LOG_LAYOUT,
        ...state.logLayout,
        ...action.payload,
      };
    },
    requestGitLogFilter: (
      state,
      action: PayloadAction<GitLogRequestPayload>,
    ) => {
      const { chatId, root, path, ref } = action.payload;
      const context = ensureContext(state, chatId);
      const nonce = (state.logRequestSeq ?? 0) + 1;
      state.logRequestSeq = nonce;
      context.logRequest = {
        ...(root ? { root } : {}),
        ...(path ? { path } : {}),
        ...(ref ? { ref } : {}),
        nonce,
      };
    },
    consumeGitLogRequest: (
      state,
      action: PayloadAction<ConsumeGitLogRequestPayload>,
    ) => {
      const context = state.contexts[gitPanelContextKey(action.payload.chatId)];
      if (context?.logRequest?.nonce !== action.payload.nonce) return;
      context.logRequest = null;
    },
  },
});

export const {
  consumeGitLogRequest,
  requestGitLogFilter,
  selectGitDiffTarget,
  setActiveGitRoot,
  setGitDiffViewerPrefs,
  setGitLogLayout,
  updateGitRootView,
} = gitPanelSlice.actions;

type GitPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof bindSurfaceToChat>
    | ReturnType<typeof selectGitDiffTarget>
    | ReturnType<typeof setDockOpen>
    | ReturnType<typeof setActiveGitRoot>
    | ReturnType<typeof setGitLogLayout>
    | ReturnType<typeof requestGitLogFilter>,
) => void;

type GitPanelThunkState = {
  workspace: WorkspaceState;
};

/**
 * Working-tree diff: HEAD (old path for renames) against the working tree.
 * Untracked/added files have no left side, deleted files no right side;
 * conflicted files compare HEAD with the conflicted working copy.
 */
export function workingDiffDescriptor(
  target: GitDiffTarget,
): GitDiffDescriptor {
  const leftPath = target.old_path ?? target.path;
  const missingLeft =
    target.kind === "untracked" ||
    (target.kind === "change" && target.status === "ADDED");
  const missingRight = target.kind === "change" && target.status === "DELETED";
  return {
    root: target.root,
    path: target.path,
    old_path: target.old_path,
    left: missingLeft ? null : { rev: "HEAD", path: leftPath },
    right: missingRight ? null : { rev: "WORKTREE", path: target.path },
    kind: "working",
  };
}

/** Commit diff: first parent (old path for renames) against the commit. */
export function commitDiffDescriptor(
  target: GitCommitDiffTarget,
): GitDiffDescriptor {
  const leftPath = target.old_path ?? target.path;
  const missingLeft = target.parent_oid === null || target.status === "ADDED";
  const missingRight = target.status === "DELETED";
  return {
    root: target.root,
    path: target.path,
    old_path: target.old_path,
    left:
      missingLeft || target.parent_oid === null
        ? null
        : { rev: target.parent_oid, path: leftPath },
    right: missingRight ? null : { rev: target.commit.oid, path: target.path },
    kind: "commit",
    commit: target.commit,
  };
}

function openGitSurface(
  surfaceKey: SurfaceKey,
  dispatch: GitPanelDispatch,
  chatId: string | null,
) {
  dispatch(openTab(surfaceKey));
  if (chatId) dispatch(bindSurfaceToChat({ surfaceKey, chatId }));
  if (
    typeof window !== "undefined" &&
    window.matchMedia("(max-width: 767px)").matches
  ) {
    dispatch(setDockOpen(false));
  }
  return surfaceKey;
}

function openDiffSurface(
  descriptor: GitDiffDescriptor,
  dispatch: GitPanelDispatch,
  chatId: string | null,
) {
  return openGitSurface(makeGitDiffSurfaceKey(descriptor), dispatch, chatId);
}

export const GIT_LOG_SURFACE_KEY: SurfaceKey = "git:main";

/**
 * Opens the Git tool window on its Log tab. A `path` or `ref` becomes a
 * one-shot filter request the mounted GitLog applies (once per nonce).
 */
export const openGitLog =
  (options: OpenGitLogOptions = {}) =>
  (dispatch: GitPanelDispatch, getState: () => GitPanelThunkState) => {
    const chatId = selectFocusedWorkspaceChatId(getState());
    if (options.root) {
      dispatch(setActiveGitRoot({ chatId, root: options.root }));
    }
    dispatch(setGitLogLayout({ tab: "log" }));
    if (Boolean(options.path) || Boolean(options.ref)) {
      dispatch(requestGitLogFilter({ chatId, ...options }));
    }
    return openGitSurface(GIT_LOG_SURFACE_KEY, dispatch, chatId);
  };

export const openGitDiff =
  (target: GitDiffTarget) =>
  (dispatch: GitPanelDispatch, getState: () => GitPanelThunkState) => {
    const chatId = selectFocusedWorkspaceChatId(getState());
    dispatch(selectGitDiffTarget({ chatId, selection: target }));
    return openDiffSurface(workingDiffDescriptor(target), dispatch, chatId);
  };

export const openGitCommitDiff =
  (target: GitCommitDiffTarget) =>
  (dispatch: GitPanelDispatch, getState: () => GitPanelThunkState) => {
    const chatId = selectFocusedWorkspaceChatId(getState());
    return openDiffSurface(commitDiffDescriptor(target), dispatch, chatId);
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

export const selectGitLogRequest = (
  state: GitPanelRootState,
  chatId: string | null,
): GitLogRequestState | null =>
  selectGitPanelContext(state, chatId).logRequest ?? null;

export const selectGitDiffViewerPrefs = (
  state: GitPanelRootState,
): GitDiffViewerPrefs =>
  state.gitPanel.diffViewer ?? DEFAULT_GIT_DIFF_VIEWER_PREFS;

export const selectGitLogLayout = (state: GitPanelRootState): GitLogLayout =>
  state.gitPanel.logLayout ?? DEFAULT_GIT_LOG_LAYOUT;

export default gitPanelSlice.reducer;
