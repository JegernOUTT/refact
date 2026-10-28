import type {
  GitBranchesRoot,
  GitRef,
  GitRefKind,
} from "../../../../services/refact/gitRead";
import { sortRefs } from "./sortRefs";

export const MAX_VISIBLE_REF_LABELS = 3;

export type RefLabelContext = {
  remotes: readonly string[];
  currentBranch: string | null;
  headDetached: boolean;
};

export const EMPTY_REF_LABEL_CONTEXT: RefLabelContext = {
  remotes: [],
  currentBranch: null,
  headDetached: false,
};

export type RefLabel = {
  key: string;
  kind: GitRefKind;
  text: string;
  /** Full ref names represented by this label. */
  names: string[];
  current: boolean;
};

export type RefLabelGroups = {
  visible: RefLabel[];
  hidden: RefLabel[];
};

export function refLabelContext(
  data: GitBranchesRoot | undefined,
): RefLabelContext {
  if (!data) return EMPTY_REF_LABEL_CONTEXT;
  const remotes = new Set(data.remotes ?? []);
  for (const branch of data.remote_branches ?? []) remotes.add(branch.remote);
  const headDetached = data.head_detached === true;
  const currentBranch = headDetached
    ? null
    : data.current ??
      data.branches.find((branch) => branch.is_head)?.name ??
      null;
  return { remotes: [...remotes], currentBranch, headDetached };
}

function splitRemote(
  name: string,
  remotes: readonly string[],
): { remote: string; branch: string } | null {
  let best: string | null = null;
  for (const remote of remotes) {
    if (!name.startsWith(`${remote}/`) || name.length <= remote.length + 1) {
      continue;
    }
    if (best === null || remote.length > best.length) best = remote;
  }
  if (best === null) return null;
  return { remote: best, branch: name.slice(best.length + 1) };
}

type BranchGroup = {
  branch: string;
  local: GitRef | null;
  remotes: { remote: string; ref: GitRef }[];
};

function groupLabel(group: BranchGroup, currentBranch: string | null) {
  const remoteNames = group.remotes.map(({ remote }) => remote);
  const names = [
    ...(group.local ? [group.local.name] : []),
    ...group.remotes.map(({ ref }) => ref.name),
  ];
  const merged = group.local !== null || group.remotes.length > 1;
  const text = merged
    ? [...remoteNames, group.branch].join(" & ")
    : group.remotes[0].ref.name;
  const current = group.local !== null && group.local.name === currentBranch;
  const kind: GitRefKind = group.local ? "local" : "remote";
  return { key: `branch:${group.branch}`, kind, text, names, current };
}

export function buildRefLabels(
  refs: readonly GitRef[],
  context: RefLabelContext,
): RefLabel[] {
  const sorted = sortRefs(refs);
  const groups = new Map<string, BranchGroup>();
  const group = (branch: string) => {
    let entry = groups.get(branch);
    if (!entry) {
      entry = { branch, local: null, remotes: [] };
      groups.set(branch, entry);
    }
    return entry;
  };
  const heads: GitRef[] = [];
  const singles: RefLabel[] = [];
  for (const ref of sorted) {
    if (ref.kind === "head") {
      heads.push(ref);
    } else if (ref.kind === "local") {
      group(ref.name).local = ref;
    } else if (ref.kind === "remote") {
      const split = splitRemote(ref.name, context.remotes);
      if (split)
        group(split.branch).remotes.push({ remote: split.remote, ref });
      else singles.push(single(ref));
    } else {
      singles.push(single(ref));
    }
  }

  const branchLabels = [...groups.values()].map((entry) =>
    groupLabel(entry, context.currentBranch),
  );
  const attached =
    !context.headDetached && branchLabels.some((label) => label.current);
  const headLabels = attached ? [] : heads.map(single);
  const current = branchLabels.filter((label) => label.current);
  const locals = branchLabels.filter(
    (label) => !label.current && label.kind === "local",
  );
  const remotes = branchLabels.filter((label) => label.kind === "remote");
  const singleRemotes = singles.filter((label) => label.kind === "remote");
  const tags = singles.filter((label) => label.kind === "tag");
  return [
    ...current,
    ...headLabels,
    ...locals,
    ...remotes,
    ...singleRemotes,
    ...tags,
  ];
}

function single(ref: GitRef): RefLabel {
  return {
    key: `${ref.kind}:${ref.name}`,
    kind: ref.kind,
    text: ref.name,
    names: [ref.name],
    current: false,
  };
}

export function groupRefLabels(
  refs: readonly GitRef[],
  context: RefLabelContext,
  maxVisible: number = MAX_VISIBLE_REF_LABELS,
): RefLabelGroups {
  const labels = buildRefLabels(refs, context);
  return {
    visible: labels.slice(0, maxVisible),
    hidden: labels.slice(maxVisible),
  };
}
