import { describe, expect, it } from "vitest";
import { setUpStore, type RootState } from "../app/store";
import {
  currentProjectInfoReducer,
  markProjectBuddySnapshotReceived,
  markProjectHistorySnapshotReceived,
  markProjectServerSnapshotReceived,
  markProjectTasksSnapshotReceived,
  resetProjectServerSnapshot,
  selectHasBuddySnapshot,
  selectHasHistorySnapshot,
  selectHasProjectSnapshot,
  selectHasTasksSnapshot,
  setCurrentProjectInfo,
} from "../features/Chat/currentProject";

function makeState(current_project: RootState["current_project"]): RootState {
  return setUpStore({ current_project }).getState();
}

describe("current project server snapshot readiness", () => {
  it("starts without a received server snapshot", () => {
    const state = setUpStore().getState();

    expect(selectHasProjectSnapshot(state)).toBe(false);
  });

  it("marks and resets server snapshot readiness", () => {
    const ready = currentProjectInfoReducer(
      {
        name: "repo",
        workspaceRoots: ["/tmp/repo"],
        serverSnapshotReceived: false,
      },
      markProjectServerSnapshotReceived(),
    );

    expect(selectHasProjectSnapshot(makeState(ready))).toBe(true);

    const withSections = [
      markProjectHistorySnapshotReceived(),
      markProjectTasksSnapshotReceived(),
      markProjectBuddySnapshotReceived(),
    ].reduce(currentProjectInfoReducer, ready);
    const sectionState = makeState(withSections);

    expect(selectHasHistorySnapshot(sectionState)).toBe(true);
    expect(selectHasTasksSnapshot(sectionState)).toBe(true);
    expect(selectHasBuddySnapshot(sectionState)).toBe(true);

    const reset = currentProjectInfoReducer(
      withSections,
      resetProjectServerSnapshot(),
    );
    const resetState = makeState(reset);

    expect(selectHasProjectSnapshot(resetState)).toBe(false);
    expect(selectHasHistorySnapshot(resetState)).toBe(false);
    expect(selectHasTasksSnapshot(resetState)).toBe(false);
    expect(selectHasBuddySnapshot(resetState)).toBe(false);
  });

  it("preserves readiness across same-project IDE updates", () => {
    const ready = currentProjectInfoReducer(
      {
        name: "repo",
        workspaceRoots: ["/tmp/repo"],
        serverSnapshotReceived: true,
      },
      setCurrentProjectInfo({ name: "repo" }),
    );

    expect(ready.serverSnapshotReceived).toBe(true);
  });

  it("resets readiness when project identity changes", () => {
    const changed = currentProjectInfoReducer(
      {
        name: "repo",
        workspaceRoots: ["/tmp/repo"],
        serverSnapshotReceived: true,
      },
      setCurrentProjectInfo({ name: "other", workspaceRoots: ["/tmp/other"] }),
    );

    expect(changed.serverSnapshotReceived).toBe(false);
  });

  it("accepts explicit snapshot readiness from server updates", () => {
    const ready = currentProjectInfoReducer(
      { name: "", serverSnapshotReceived: false },
      setCurrentProjectInfo({
        name: "repo",
        workspaceRoots: ["/tmp/repo"],
        serverSnapshotReceived: true,
      }),
    );

    expect(ready.serverSnapshotReceived).toBe(true);
  });
});
