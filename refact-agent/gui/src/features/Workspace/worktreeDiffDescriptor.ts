export type WorktreeDiffDescriptor = {
  worktreeId: string;
  sourceWorkspaceRoot: string;
  root?: string;
  label?: string;
};

export function encodeWorktreeDiffDescriptor(
  value: WorktreeDiffDescriptor,
): string {
  return encodeURIComponent(
    JSON.stringify({
      worktreeId: value.worktreeId,
      sourceWorkspaceRoot: value.sourceWorkspaceRoot,
      ...(value.root ? { root: value.root } : {}),
      ...(value.label ? { label: value.label } : {}),
    }),
  );
}

export function decodeWorktreeDiffDescriptor(
  encoded: string,
): WorktreeDiffDescriptor | null {
  try {
    const value: unknown = JSON.parse(decodeURIComponent(encoded));
    if (!value || typeof value !== "object" || Array.isArray(value))
      return null;
    const record = value as Record<string, unknown>;
    if (
      typeof record.worktreeId !== "string" ||
      !record.worktreeId ||
      typeof record.sourceWorkspaceRoot !== "string" ||
      !record.sourceWorkspaceRoot ||
      (record.root !== undefined && typeof record.root !== "string") ||
      (record.label !== undefined && typeof record.label !== "string")
    )
      return null;
    return {
      worktreeId: record.worktreeId,
      sourceWorkspaceRoot: record.sourceWorkspaceRoot,
      ...(typeof record.root === "string" ? { root: record.root } : {}),
      ...(typeof record.label === "string" ? { label: record.label } : {}),
    };
  } catch {
    return null;
  }
}
