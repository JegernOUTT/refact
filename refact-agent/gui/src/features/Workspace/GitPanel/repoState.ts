import type { GitRepoState } from "../../../services/refact/gitRead";

export const REPO_STATE_LABELS: Partial<Record<GitRepoState, string>> = {
  merge: "Merging",
  rebase: "Rebasing",
  cherry_pick: "Cherry-picking",
  revert: "Reverting",
  bisect: "Bisecting",
  apply_mailbox: "Applying patches",
  other: "Operation in progress",
};

export function repoBlockedReason(
  state: GitRepoState | null | undefined,
): string | null {
  if (!state || state === "clean") return null;
  const label = REPO_STATE_LABELS[state] ?? "An operation is in progress";
  return `${label} — finish or abort it before updating or checking out.`;
}
