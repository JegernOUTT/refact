import { createReducer, createAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";

export type CurrentProjectInfo = {
  name: string;
  workspaceRoots?: string[];
  serverSnapshotReceived?: boolean;
  historySnapshotReceived?: boolean;
  tasksSnapshotReceived?: boolean;
  buddySnapshotReceived?: boolean;
};

const initialState: CurrentProjectInfo = {
  name: "",
  serverSnapshotReceived: false,
  historySnapshotReceived: false,
  tasksSnapshotReceived: false,
  buddySnapshotReceived: false,
};

export const setCurrentProjectInfo = createAction<CurrentProjectInfo>(
  "currentProjectInfo/setCurrentProjectInfo",
);

export const markProjectServerSnapshotReceived = createAction(
  "currentProjectInfo/markProjectServerSnapshotReceived",
);

export const markProjectHistorySnapshotReceived = createAction(
  "currentProjectInfo/markProjectHistorySnapshotReceived",
);

export const markProjectTasksSnapshotReceived = createAction(
  "currentProjectInfo/markProjectTasksSnapshotReceived",
);

export const markProjectBuddySnapshotReceived = createAction(
  "currentProjectInfo/markProjectBuddySnapshotReceived",
);

export const resetProjectServerSnapshot = createAction(
  "currentProjectInfo/resetProjectServerSnapshot",
);

function sameWorkspaceRoots(left?: string[], right?: string[]): boolean {
  if (left === undefined || right === undefined) return false;
  if (left.length !== right.length) return false;
  return left.every((root, index) => root === right[index]);
}

function isSameProjectIdentity(
  current: CurrentProjectInfo,
  next: CurrentProjectInfo,
): boolean {
  if (sameWorkspaceRoots(current.workspaceRoots, next.workspaceRoots)) {
    return true;
  }
  if (next.workspaceRoots !== undefined) return false;

  return Boolean(
    current.name.trim() && current.name.trim() === next.name.trim(),
  );
}

export const currentProjectInfoReducer = createReducer(
  initialState,
  (builder) => {
    builder
      .addCase(setCurrentProjectInfo, (state, action) => {
        const explicitSnapshot = action.payload.serverSnapshotReceived;
        const shouldPreserveSnapshot =
          explicitSnapshot === undefined &&
          isSameProjectIdentity(state, action.payload);

        const preserve = shouldPreserveSnapshot;

        return {
          ...action.payload,
          serverSnapshotReceived:
            explicitSnapshot ??
            (preserve ? Boolean(state.serverSnapshotReceived) : false),
          historySnapshotReceived:
            action.payload.historySnapshotReceived ??
            (preserve ? Boolean(state.historySnapshotReceived) : false),
          tasksSnapshotReceived:
            action.payload.tasksSnapshotReceived ??
            (preserve ? Boolean(state.tasksSnapshotReceived) : false),
          buddySnapshotReceived:
            action.payload.buddySnapshotReceived ??
            (preserve ? Boolean(state.buddySnapshotReceived) : false),
        };
      })
      .addCase(markProjectServerSnapshotReceived, (state) => {
        state.serverSnapshotReceived = true;
      })
      .addCase(markProjectHistorySnapshotReceived, (state) => {
        state.historySnapshotReceived = true;
      })
      .addCase(markProjectTasksSnapshotReceived, (state) => {
        state.tasksSnapshotReceived = true;
      })
      .addCase(markProjectBuddySnapshotReceived, (state) => {
        state.buddySnapshotReceived = true;
      })
      .addCase(resetProjectServerSnapshot, (state) => {
        state.serverSnapshotReceived = false;
        state.historySnapshotReceived = false;
        state.tasksSnapshotReceived = false;
        state.buddySnapshotReceived = false;
      });
  },
);

export const selectThreadProjectOrCurrentProject = (state: RootState) => {
  const threadId = state.chat.current_thread_id;
  const runtime = threadId ? state.chat.threads[threadId] : undefined;
  if (!runtime) {
    return state.current_project.name;
  }
  const thread = runtime.thread;
  if (thread.integration?.project) {
    return thread.integration.project;
  }
  return thread.project_name ?? state.current_project.name;
};

export const selectHasActiveProject = (state: RootState): boolean => {
  const workspaceRoots = state.current_project.workspaceRoots;
  if (workspaceRoots !== undefined) {
    return workspaceRoots.length > 0;
  }

  return Boolean(
    state.current_project.name.trim() ||
      state.config.currentWorkspaceName?.trim(),
  );
};

export const selectHasProjectSnapshot = (state: RootState): boolean =>
  Boolean(state.current_project.serverSnapshotReceived);

export const selectHasHistorySnapshot = (state: RootState): boolean =>
  Boolean(state.current_project.historySnapshotReceived);

export const selectHasTasksSnapshot = (state: RootState): boolean =>
  Boolean(state.current_project.tasksSnapshotReceived);

export const selectHasBuddySnapshot = (state: RootState): boolean =>
  Boolean(state.current_project.buddySnapshotReceived);
