import { createReducer, createAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";

export type CurrentProjectInfo = {
  name: string;
  workspaceRoots?: string[];
  workspaceSnapshotReceived?: boolean;
  trajectoriesSnapshotReceived?: boolean;
  tasksSnapshotReceived?: boolean;
  buddySnapshotReceived?: boolean;
};

const initialState: CurrentProjectInfo = {
  name: "",
  workspaceSnapshotReceived: false,
  trajectoriesSnapshotReceived: false,
  tasksSnapshotReceived: false,
  buddySnapshotReceived: false,
};

export const setCurrentProjectInfo = createAction<CurrentProjectInfo>(
  "currentProjectInfo/setCurrentProjectInfo",
);

export const markWorkspaceSnapshotReceived = createAction(
  "currentProjectInfo/markWorkspaceSnapshotReceived",
);

export const markTrajectoriesSnapshotReceived = createAction(
  "currentProjectInfo/markTrajectoriesSnapshotReceived",
);

export const markTasksSnapshotReceived = createAction(
  "currentProjectInfo/markTasksSnapshotReceived",
);

export const markBuddySnapshotReceived = createAction(
  "currentProjectInfo/markBuddySnapshotReceived",
);

export const resetSidebarReadiness = createAction(
  "currentProjectInfo/resetSidebarReadiness",
);

type ProjectIdentity = Pick<CurrentProjectInfo, "name" | "workspaceRoots">;

function sameStringArray(left?: string[], right?: string[]): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  if (left.length !== right.length) return false;
  return left.every((item, index) => item === right[index]);
}

function shouldPreserveWorkspaceRoots(
  state: CurrentProjectInfo,
  next: CurrentProjectInfo,
): boolean {
  if (!state.workspaceRoots || next.workspaceRoots !== undefined) return false;

  const nextName = next.name.trim();
  if (!nextName) return false;

  return nextName === state.name;
}

function hasWorkspaceIdentityChanged(
  current: ProjectIdentity,
  next: CurrentProjectInfo,
): boolean {
  if (next.workspaceRoots !== undefined) {
    if (current.workspaceRoots === undefined) {
      return Boolean(
        current.name.trim() &&
          next.name.trim() &&
          current.name.trim() !== next.name.trim(),
      );
    }
    return !sameStringArray(current.workspaceRoots, next.workspaceRoots);
  }

  return next.name.trim() !== current.name.trim();
}

export const currentProjectInfoReducer = createReducer(
  initialState,
  (builder) => {
    builder
      .addCase(setCurrentProjectInfo, (state, action) => {
        const next = action.payload;
        const identityChanged = hasWorkspaceIdentityChanged(state, next);
        const nextRoots =
          next.workspaceRoots ??
          (shouldPreserveWorkspaceRoots(state, next)
            ? state.workspaceRoots
            : undefined);

        state.name = next.name;
        if (nextRoots !== undefined) {
          state.workspaceRoots = nextRoots;
        } else {
          delete state.workspaceRoots;
        }

        if (identityChanged) {
          state.workspaceSnapshotReceived = false;
          state.trajectoriesSnapshotReceived = false;
          state.tasksSnapshotReceived = false;
          state.buddySnapshotReceived = false;
        }

        if (next.workspaceSnapshotReceived !== undefined) {
          state.workspaceSnapshotReceived = next.workspaceSnapshotReceived;
        }
        if (next.trajectoriesSnapshotReceived !== undefined) {
          state.trajectoriesSnapshotReceived =
            next.trajectoriesSnapshotReceived;
        }
        if (next.tasksSnapshotReceived !== undefined) {
          state.tasksSnapshotReceived = next.tasksSnapshotReceived;
        }
        if (next.buddySnapshotReceived !== undefined) {
          state.buddySnapshotReceived = next.buddySnapshotReceived;
        }
      })
      .addCase(markWorkspaceSnapshotReceived, (state) => {
        state.workspaceSnapshotReceived = true;
      })
      .addCase(markTrajectoriesSnapshotReceived, (state) => {
        state.trajectoriesSnapshotReceived = true;
      })
      .addCase(markTasksSnapshotReceived, (state) => {
        state.tasksSnapshotReceived = true;
      })
      .addCase(markBuddySnapshotReceived, (state) => {
        state.buddySnapshotReceived = true;
      })
      .addCase(resetSidebarReadiness, (state) => {
        state.workspaceSnapshotReceived = false;
        state.trajectoriesSnapshotReceived = false;
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

export const selectWorkspaceSnapshotReceived = (state: RootState): boolean =>
  state.current_project.workspaceSnapshotReceived === true;

export const selectTrajectoriesSnapshotReceived = (state: RootState): boolean =>
  state.current_project.trajectoriesSnapshotReceived === true;

export const selectTasksSnapshotReceived = (state: RootState): boolean =>
  state.current_project.tasksSnapshotReceived === true;

export const selectBuddySnapshotReceived = (state: RootState): boolean =>
  state.current_project.buddySnapshotReceived === true;
