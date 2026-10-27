import type { GitBranchesRoot } from "../../../../services/refact/gitRead";
import {
  allBranchRefs,
  buildBranchTree,
  type BranchRef,
} from "../log/branchesTreeModel";
import { remoteOfRef } from "./branchName";
import type { GitMenuAction } from "./menuActions";

export type PopupAction = GitMenuAction & { icon: PopupActionIcon };

export type PopupActionIcon =
  | "update"
  | "commit"
  | "push"
  | "new-branch"
  | "checkout-revision"
  | "fetch";

export type PopupRow =
  | { kind: "header"; id: string; label: string; depth: number }
  | { kind: "action"; id: string; action: PopupAction }
  | { kind: "branch"; id: string; ref: BranchRef; depth: number }
  | {
      kind: "branchAction";
      id: string;
      parentId: string;
      action: GitMenuAction;
      depth: number;
    };

export function isSelectable(row: PopupRow): boolean {
  if (row.kind === "header") return false;
  if (row.kind === "action" || row.kind === "branchAction") {
    return !row.action.disabled;
  }
  return true;
}

const branchRowId = (ref: BranchRef) => `branch:${ref.kind}:${ref.name}`;

function matches(text: string, query: string): boolean {
  return text.toLowerCase().includes(query);
}

type BuildOptions = {
  actions: PopupAction[];
  branches: GitBranchesRoot | undefined;
  remotes: readonly string[];
  query: string;
  expandedId: string | null;
  branchActions: (ref: BranchRef) => GitMenuAction[];
};

export function buildPopupRows({
  actions,
  branches,
  remotes,
  query,
  expandedId,
  branchActions,
}: BuildOptions): PopupRow[] {
  const needle = query.trim().toLowerCase();
  const rows: PopupRow[] = [];
  const visibleActions = actions.filter(
    (action) => !needle || matches(action.label, needle),
  );
  if (visibleActions.length > 0) {
    rows.push({
      kind: "header",
      id: "header:actions",
      label: "Actions",
      depth: 0,
    });
    for (const action of visibleActions) {
      rows.push({ kind: "action", id: `action:${action.id}`, action });
    }
  }

  const refs = branches ? allBranchRefs(buildBranchTree(branches)) : [];
  const visibleRefs = refs.filter(
    (ref) => ref.kind !== "tag" && (!needle || matches(ref.name, needle)),
  );

  const pushBranch = (ref: BranchRef, depth: number) => {
    const id = branchRowId(ref);
    rows.push({ kind: "branch", id, ref, depth });
    if (expandedId !== id) return;
    for (const action of branchActions(ref)) {
      rows.push({
        kind: "branchAction",
        id: `${id}:${action.id}`,
        parentId: id,
        action,
        depth: depth + 1,
      });
    }
  };

  const locals = visibleRefs
    .filter((ref) => ref.kind === "local")
    .sort((left, right) => Number(right.current) - Number(left.current));
  if (locals.length > 0) {
    rows.push({ kind: "header", id: "header:local", label: "Local", depth: 0 });
    for (const ref of locals) pushBranch(ref, 0);
  }

  const remoteRefs = visibleRefs.filter((ref) => ref.kind === "remote");
  if (remoteRefs.length > 0) {
    rows.push({
      kind: "header",
      id: "header:remote",
      label: "Remote",
      depth: 0,
    });
    const groups = new Map<string, BranchRef[]>();
    for (const ref of remoteRefs) {
      const remote = remoteOfRef(ref.name, remotes)?.remote ?? "";
      groups.set(remote, [...(groups.get(remote) ?? []), ref]);
    }
    for (const [remote, groupRefs] of groups) {
      rows.push({
        kind: "header",
        id: `header:remote:${remote}`,
        label: remote,
        depth: 1,
      });
      for (const ref of groupRefs) pushBranch(ref, 1);
    }
  }
  return rows;
}
