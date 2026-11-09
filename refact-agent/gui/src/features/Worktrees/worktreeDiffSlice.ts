import type { AppDispatch, RootState } from "../../app/store";
import {
  makeWorktreeDiffSurfaceKey,
  worktreeDiffDescriptorFromSurfaceKey,
} from "../Workspace/surfaceKey";
import { openSurfaceInNewTab, setDockOpen } from "../Workspace/workspaceSlice";
import { collectTabIds } from "../ChatPanes/panesTree";
import type { WorktreeDiffDescriptor } from "../Workspace/worktreeDiffDescriptor";

export const openWorktreeDiff =
  (descriptor: WorktreeDiffDescriptor) =>
  (dispatch: AppDispatch, getState: () => RootState) => {
    const workspace = getState().workspace;
    const keys = [
      ...workspace.tabs,
      ...Object.values(workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ];
    const existing = keys.find((key) => {
      const value = worktreeDiffDescriptorFromSurfaceKey(key);
      return (
        value?.worktreeId === descriptor.worktreeId &&
        value.sourceWorkspaceRoot === descriptor.sourceWorkspaceRoot
      );
    });
    dispatch(
      openSurfaceInNewTab(existing ?? makeWorktreeDiffSurfaceKey(descriptor)),
    );
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(max-width: 767px)").matches
    )
      dispatch(setDockOpen(false));
  };
