import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { DiffChunk } from "../../../services/refact";
import {
  getProjectStorageNamespace,
  isProjectStorageNamespaceTrusted,
} from "../../../utils/chatUiPersistence";

import { makeSurfaceKey } from "../surfaceKey";
import type { WorkspaceState } from "../workspaceSlice";
import {
  bindSurfaceToChat,
  openTab,
  selectFocusedWorkspaceChatId,
  selectFocusedChatWorktreeRoot,
  setDockOpen,
} from "../workspaceSlice";

export type FileViewerTarget = {
  path: string;
  line?: number;
};

export type LiveFileUpdate = {
  revision: string;
  chunks: DiffChunk[];
  operation: "write" | "remove" | "rename";
  renamedTo?: string;
  authoritative: boolean;
};

export type FilesPanelState = {
  expandedDirectories: string[];
  selectedPath: string | null;
  showIgnored: boolean;
  viewerTarget: FileViewerTarget | null;
  viewerTargets: Record<string, FileViewerTarget | undefined>;
  liveUpdatesByChat: Record<
    string,
    Record<string, LiveFileUpdate | undefined> | undefined
  >;
};

const initialState: FilesPanelState = {
  expandedDirectories: [],
  selectedPath: null,
  showIgnored: false,
  viewerTarget: null,
  viewerTargets: {},
  liveUpdatesByChat: {},
};

const compareRevision = (left: string, right: string): number => {
  if (/^\d+$/u.test(left) && /^\d+$/u.test(right)) {
    const leftValue = BigInt(left);
    const rightValue = BigInt(right);
    return leftValue === rightValue ? 0 : leftValue > rightValue ? 1 : -1;
  }
  return left.localeCompare(right);
};

const SHOW_IGNORED_STORAGE_KEY = "refact:files-panel:show-ignored:v1";

const showIgnoredStorageKey = (): string | null => {
  const namespace = getProjectStorageNamespace();
  return isProjectStorageNamespaceTrusted() && namespace
    ? `refact:project:${namespace}:${SHOW_IGNORED_STORAGE_KEY}`
    : null;
};

export const loadPersistedShowIgnored = (): boolean => {
  const key = showIgnoredStorageKey();
  if (!key || typeof localStorage === "undefined") return false;
  try {
    return localStorage.getItem(key) === "true";
  } catch {
    return false;
  }
};

const persistShowIgnored = (showIgnored: boolean): void => {
  const key = showIgnoredStorageKey();
  if (!key || typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(key, String(showIgnored));
  } catch {
    return;
  }
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
    resetFileTree: (state) => {
      state.expandedDirectories = [];
      state.selectedPath = null;
    },
    hydrateShowIgnored: (state, action: PayloadAction<boolean>) => {
      state.showIgnored = action.payload;
    },
    setShowIgnored: (state, action: PayloadAction<boolean>) => {
      state.showIgnored = action.payload;
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
    applyLiveFileUpdate: (
      state,
      action: PayloadAction<{
        chatId: string;
        path: string;
        update: Omit<LiveFileUpdate, "authoritative">;
      }>,
    ) => {
      const updates = state.liveUpdatesByChat[action.payload.chatId] ?? {};
      const latest = updates[action.payload.path];
      if (
        latest &&
        compareRevision(action.payload.update.revision, latest.revision) <= 0
      ) {
        return;
      }
      state.liveUpdatesByChat[action.payload.chatId] = {
        ...updates,
        [action.payload.path]: {
          ...action.payload.update,
          authoritative: false,
        },
      };
    },
    markLiveFileUpdateAuthoritative: (
      state,
      action: PayloadAction<{
        chatId: string;
        path: string;
        revision: string;
      }>,
    ) => {
      const update =
        state.liveUpdatesByChat[action.payload.chatId]?.[action.payload.path];
      if (update?.revision === action.payload.revision) {
        update.authoritative = true;
      }
    },
    clearLiveFileUpdate: (
      state,
      action: PayloadAction<{
        chatId: string;
        path: string;
        revision?: string;
      }>,
    ) => {
      const updates = state.liveUpdatesByChat[action.payload.chatId];
      const update = updates?.[action.payload.path];
      if (!update) return;
      if (
        action.payload.revision &&
        update.revision !== action.payload.revision
      ) {
        return;
      }
      const { [action.payload.path]: _removed, ...remaining } = updates;
      if (Object.keys(remaining).length > 0) {
        state.liveUpdatesByChat[action.payload.chatId] = remaining;
      } else {
        const { [action.payload.chatId]: _chat, ...otherChats } =
          state.liveUpdatesByChat;
        state.liveUpdatesByChat = otherChats;
      }
    },
    clearLiveFileUpdatesForChat: (state, action: PayloadAction<string>) => {
      const { [action.payload]: _chat, ...otherChats } =
        state.liveUpdatesByChat;
      state.liveUpdatesByChat = otherChats;
    },
  },
});

export const {
  clearLiveFileUpdate,
  clearLiveFileUpdatesForChat,
  collapseDirectory,
  applyLiveFileUpdate,
  expandDirectory,
  hydrateShowIgnored,
  markLiveFileUpdateAuthoritative,
  resetFileTree,
  selectTreePath,
  setShowIgnored,
  setViewerTarget,
  toggleDirectory,
} = filesPanelSlice.actions;

export const updateShowIgnored =
  (showIgnored: boolean) =>
  (dispatch: (action: ReturnType<typeof setShowIgnored>) => void) => {
    dispatch(setShowIgnored(showIgnored));
    persistShowIgnored(showIgnored);
  };

type FilesPanelDispatch = (
  action:
    | ReturnType<typeof openTab>
    | ReturnType<typeof bindSurfaceToChat>
    | ReturnType<typeof expandDirectory>
    | ReturnType<typeof resetFileTree>
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
  workspace: WorkspaceState;
  chat: {
    threads: Record<
      string,
      | {
          thread: {
            worktree?: {
              root: string;
              source_workspace_root: string;
            } | null;
          };
        }
      | undefined
    >;
  };
  current_project: {
    workspaceRoots?: string[];
  };
};

export const openFileInFilesPanel =
  (target: FileViewerTarget) =>
  (dispatch: FilesPanelDispatch, getState: () => FilesPanelThunkState) => {
    const state = getState();
    const chatId = selectFocusedWorkspaceChatId(state);
    const worktreeRoot = selectFocusedChatWorktreeRoot(state);
    const workspaceRoots = worktreeRoot
      ? [worktreeRoot]
      : state.current_project.workspaceRoots ?? [];
    const surfaceKey = makeSurfaceKey("file", target.path);
    dispatch(openTab(surfaceKey));
    if (chatId) dispatch(bindSurfaceToChat({ surfaceKey, chatId }));
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

export const selectShowIgnored = (state: FilesPanelRootState) =>
  state.filesPanel.showIgnored;

export const selectFileViewerTarget = (state: FilesPanelRootState) =>
  state.filesPanel.viewerTarget;

export const selectFileViewerTargetByPath = (
  state: FilesPanelRootState,
  path: string,
): FileViewerTarget | undefined => state.filesPanel.viewerTargets[path];

export const selectLiveFileUpdate = (
  state: FilesPanelRootState,
  chatId: string | null,
  path: string,
): LiveFileUpdate | undefined =>
  chatId ? state.filesPanel.liveUpdatesByChat[chatId]?.[path] : undefined;
