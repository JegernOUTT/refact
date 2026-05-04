import { describe, expect, it } from "vitest";
import {
  currentProjectInfoReducer,
  markBuddySnapshotReceived,
  markTasksSnapshotReceived,
  markTrajectoriesSnapshotReceived,
  markWorkspaceSnapshotReceived,
  resetSidebarReadiness,
  setCurrentProjectInfo,
} from "../features/Chat/currentProject";

describe("currentProjectInfoReducer", () => {
  it("tracks and resets progressive sidebar readiness", () => {
    let state = currentProjectInfoReducer(
      undefined,
      markWorkspaceSnapshotReceived(),
    );
    state = currentProjectInfoReducer(
      state,
      markTrajectoriesSnapshotReceived(),
    );
    state = currentProjectInfoReducer(state, markTasksSnapshotReceived());
    state = currentProjectInfoReducer(state, markBuddySnapshotReceived());

    expect(state.workspaceSnapshotReceived).toBe(true);
    expect(state.trajectoriesSnapshotReceived).toBe(true);
    expect(state.tasksSnapshotReceived).toBe(true);
    expect(state.buddySnapshotReceived).toBe(true);

    state = currentProjectInfoReducer(state, resetSidebarReadiness());

    expect(state.workspaceSnapshotReceived).toBe(false);
    expect(state.trajectoriesSnapshotReceived).toBe(false);
    expect(state.tasksSnapshotReceived).toBe(false);
    expect(state.buddySnapshotReceived).toBe(false);
  });

  it("preserves workspace roots when the same project update omits roots", () => {
    let state = currentProjectInfoReducer(
      undefined,
      setCurrentProjectInfo({
        name: "refact",
        workspaceRoots: ["/tmp/a/refact"],
        workspaceSnapshotReceived: true,
      }),
    );

    state = currentProjectInfoReducer(
      state,
      setCurrentProjectInfo({ name: "refact" }),
    );

    expect(state.workspaceRoots).toEqual(["/tmp/a/refact"]);
    expect(state.workspaceSnapshotReceived).toBe(true);
  });

  it("uses workspace roots, not matching names, for known project identity", () => {
    let state = currentProjectInfoReducer(
      undefined,
      setCurrentProjectInfo({
        name: "refact",
        workspaceRoots: ["/tmp/a/refact"],
        workspaceSnapshotReceived: true,
        trajectoriesSnapshotReceived: true,
        tasksSnapshotReceived: true,
        buddySnapshotReceived: true,
      }),
    );

    state = currentProjectInfoReducer(
      state,
      setCurrentProjectInfo({
        name: "refact",
        workspaceRoots: ["/tmp/b/refact"],
        workspaceSnapshotReceived: true,
      }),
    );

    expect(state.workspaceRoots).toEqual(["/tmp/b/refact"]);
    expect(state.workspaceSnapshotReceived).toBe(true);
    expect(state.trajectoriesSnapshotReceived).toBe(false);
    expect(state.tasksSnapshotReceived).toBe(false);
    expect(state.buddySnapshotReceived).toBe(false);
  });

  it("does not reset other sections when workspace roots are learned late", () => {
    let state = currentProjectInfoReducer(
      undefined,
      setCurrentProjectInfo({ name: "refact" }),
    );
    state = currentProjectInfoReducer(
      state,
      markTrajectoriesSnapshotReceived(),
    );
    state = currentProjectInfoReducer(state, markTasksSnapshotReceived());

    state = currentProjectInfoReducer(
      state,
      setCurrentProjectInfo({
        name: "refact",
        workspaceRoots: ["/tmp/a/refact"],
        workspaceSnapshotReceived: true,
      }),
    );

    expect(state.workspaceRoots).toEqual(["/tmp/a/refact"]);
    expect(state.workspaceSnapshotReceived).toBe(true);
    expect(state.trajectoriesSnapshotReceived).toBe(true);
    expect(state.tasksSnapshotReceived).toBe(true);
  });

  it("resets all sidebar readiness when workspace identity changes", () => {
    const state = currentProjectInfoReducer(
      {
        name: "refact",
        workspaceRoots: ["/tmp/a/refact"],
        workspaceSnapshotReceived: true,
        trajectoriesSnapshotReceived: true,
        tasksSnapshotReceived: true,
        buddySnapshotReceived: true,
      },
      setCurrentProjectInfo({
        name: "other-refact",
        workspaceRoots: ["/tmp/b/refact"],
      }),
    );

    expect(state.workspaceRoots).toEqual(["/tmp/b/refact"]);
    expect(state.workspaceSnapshotReceived).toBe(false);
    expect(state.trajectoriesSnapshotReceived).toBe(false);
    expect(state.tasksSnapshotReceived).toBe(false);
    expect(state.buddySnapshotReceived).toBe(false);
  });
});
