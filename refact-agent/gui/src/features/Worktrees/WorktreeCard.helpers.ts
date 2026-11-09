import type {
  WorktreeInventory,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact/worktrees";

export function sameWorktree(a?: WorktreeMeta | null, b?: WorktreeMeta | null) {
  return Boolean(
    a &&
      b &&
      a.id === b.id &&
      a.source_workspace_root === b.source_workspace_root,
  );
}

export function selectedFirst(
  records: WorktreeRecordView[],
  selected?: WorktreeMeta | null,
) {
  return [
    ...records.filter((record) => sameWorktree(record.meta, selected)),
    ...records.filter((record) => !sameWorktree(record.meta, selected)),
  ];
}

export function worktreeInspection(
  inventory: WorktreeInventory | undefined,
  meta: WorktreeMeta,
) {
  return inventory?.source_workspace_root === meta.source_workspace_root
    ? inventory.worktrees.find((entry) => entry.id === meta.id)
    : undefined;
}

export function worktreeSize(bytes?: number | null) {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0)
    return "Size unknown";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}
