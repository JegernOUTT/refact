import type {
  GitRollbackResponse,
  NormalizedGitStatus,
} from "../../../services/refact/gitRead";
import { commitPaths, type ChangesTreeEntry } from "./changesTreeModel";
import { REPO_STATE_LABELS } from "./repoState";

type RollbackRequestPlan = { paths: string[]; deleteAddedFiles: boolean };

/**
 * A staged rename is rolled back like JetBrains "moves it back": the old path
 * is restored from HEAD and the new path is unstaged *and deleted*. That
 * deletion must not depend on the "delete added files" choice, so renamed
 * targets travel in their own request.
 */
export function planRollbackRequests(
  entries: ChangesTreeEntry[],
  deleteAddedFiles: boolean,
): RollbackRequestPlan[] {
  const renamedTargets = new Set(
    entries
      .filter((entry) => entry.status === "RENAMED" && entry.oldPath)
      .map((entry) => entry.path),
  );
  const regular = commitPaths(entries).filter(
    (path) => !renamedTargets.has(path),
  );
  const plans: RollbackRequestPlan[] = [];
  if (regular.length > 0) plans.push({ paths: regular, deleteAddedFiles });
  if (renamedTargets.size > 0) {
    plans.push({ paths: [...renamedTargets], deleteAddedFiles: true });
  }
  return plans;
}

export function mergeRollbackResponses(
  responses: GitRollbackResponse[],
): GitRollbackResponse {
  const list = (value: unknown): string[] =>
    Array.isArray(value)
      ? value.filter((item): item is string => typeof item === "string")
      : [];
  return {
    restored: responses.flatMap((response) => list(response.restored)),
    unstaged: responses.flatMap((response) => list(response.unstaged)),
    deleted: responses.flatMap((response) => list(response.deleted)),
    skipped: responses.flatMap((response) => response.skipped),
  };
}

/** Number of dialog entries the engine actually rolled back. */
export function rolledBackCount(
  entries: ChangesTreeEntry[],
  response: GitRollbackResponse,
): number {
  const touched = new Set([...response.restored, ...response.unstaged]);
  return entries.filter(
    (entry) =>
      touched.has(entry.path) ||
      (entry.oldPath !== null && touched.has(entry.oldPath)),
  ).length;
}

export const ROLLBACK_EMPTY_SELECTION_HINT =
  "The selection has no changes to roll back";

export function amendBlockedReason(status: NormalizedGitStatus): string | null {
  if (status.extended && status.headOid === null) {
    return "There is no commit to amend yet.";
  }
  if (status.repoState !== "clean") {
    const label = REPO_STATE_LABELS[status.repoState] ?? "An operation";
    return `${label} is in progress — finish or abort it before amending.`;
  }
  return null;
}
