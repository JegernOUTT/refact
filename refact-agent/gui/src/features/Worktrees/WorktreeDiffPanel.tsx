import { useEffect } from "react";
import { useAppDispatch } from "../../hooks";
import type { WorktreeMeta, WorktreeRecordView } from "../../services/refact";
import { openWorktreeDiff } from "./worktreeDiffSlice";

type WorktreeDiffPanelProps = {
  open: boolean;
  worktreeId?: string | null;
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  sourceWorkspaceRoot?: string;
  onOpenChange: (open: boolean) => void;
};

export function WorktreeDiffPanel({
  open,
  worktreeId,
  worktree,
  record,
  sourceWorkspaceRoot,
  onOpenChange,
}: WorktreeDiffPanelProps) {
  const dispatch = useAppDispatch();
  const meta = record?.meta ?? worktree;
  const id = worktreeId ?? meta?.id;
  const source = sourceWorkspaceRoot ?? meta?.source_workspace_root;
  useEffect(() => {
    if (!open || !id || !source) return;
    dispatch(
      openWorktreeDiff({
        worktreeId: id,
        sourceWorkspaceRoot: source,
        root: meta?.root,
        label: meta?.branch ?? undefined,
      }),
    );
    onOpenChange(false);
  }, [dispatch, open, id, source, meta?.root, meta?.branch, onOpenChange]);
  return null;
}
