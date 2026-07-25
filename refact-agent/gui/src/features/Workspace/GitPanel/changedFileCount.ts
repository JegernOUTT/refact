import type { GitStatusRoot } from "../../../services/refact/gitRead";

export function changedFileCount(
  status: Pick<GitStatusRoot, "staged" | "unstaged">,
): number {
  return new Set(
    [...status.staged, ...status.unstaged].map(
      (change) => change.relative_path,
    ),
  ).size;
}
