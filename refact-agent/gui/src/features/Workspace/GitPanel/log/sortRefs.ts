import type { GitRef, GitRefKind } from "../../../../services/refact/gitRead";

const KIND_ORDER: Record<GitRefKind, number> = {
  head: 0,
  local: 1,
  remote: 2,
  tag: 3,
};

export function sortRefs(refs: readonly GitRef[]): GitRef[] {
  return [...refs].sort(
    (left, right) =>
      KIND_ORDER[left.kind] - KIND_ORDER[right.kind] ||
      left.name.localeCompare(right.name),
  );
}
