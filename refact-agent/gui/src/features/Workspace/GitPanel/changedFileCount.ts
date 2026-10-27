import type { GitStatusRoot } from "../../../services/refact/gitRead";

export function changedFileCount(status: GitStatusRoot): number {
  if (Array.isArray(status.changes)) {
    return (
      status.changes.length +
      (status.untracked?.length ?? 0) +
      (status.conflicted?.length ?? 0)
    );
  }
  return new Set(
    [...status.staged, ...status.unstaged].map(
      (change) => change.relative_path,
    ),
  ).size;
}
