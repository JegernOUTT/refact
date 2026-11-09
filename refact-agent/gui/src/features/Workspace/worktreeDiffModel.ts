import type {
  WorktreeDiffFile,
  WorktreeDiffResponse,
} from "../../services/refact/worktrees";
import type { GitDiffDescriptor } from "./GitPanel/diff/gitDiffDescriptor";

export const worktreeDiffEntryKey = (file: WorktreeDiffFile): string =>
  JSON.stringify([file.source, file.path]);

export function worktreeFileDescriptor(
  root: string,
  file: WorktreeDiffFile,
  data: WorktreeDiffResponse,
): GitDiffDescriptor | null {
  let leftRev: string | null;
  let rightRev: string;
  switch (file.source) {
    case "committed":
      leftRev = data.resolved_base_commit ?? data.base_commit ?? null;
      if (!leftRev) return null;
      rightRev = data.status.head_commit ?? "HEAD";
      break;
    case "staged":
      leftRev = data.status.head_commit ?? "HEAD";
      rightRev = "INDEX";
      break;
    case "unstaged":
      leftRev = "INDEX";
      rightRev = "WORKTREE";
      break;
    case "untracked":
      leftRev = null;
      rightRev = "WORKTREE";
      break;
    default:
      return null;
  }
  if ((file.status === "R" || file.status === "C") && !file.old_path)
    return null;
  return {
    root,
    path: file.path,
    old_path: file.old_path ?? null,
    left:
      file.status === "A" || leftRev === null
        ? null
        : { rev: leftRev, path: file.old_path ?? file.path },
    right: file.status === "D" ? null : { rev: rightRev, path: file.path },
    kind: "working",
  };
}
