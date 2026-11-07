export type WorkspaceFilePathResult = { path: string } | { error: string };

function isAbsoluteFilePath(path: string): boolean {
  return /^(?:\/|[A-Za-z]:[\\/]|\\\\)/.test(path);
}

export function resolveWorkspaceFilePath(
  path: string,
  workspaceRoots: readonly string[],
  worktreeRoot?: string | null,
): WorkspaceFilePathResult {
  if (isAbsoluteFilePath(path)) return { path };
  if (
    !path ||
    /[%:]/.test(path) ||
    Array.from(path).some(
      (char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
    ) ||
    path.startsWith("\\") ||
    path.split(/[\\/]/).includes("..")
  ) {
    return { error: "Cannot open this relative file path safely." };
  }
  const root =
    worktreeRoot ?? (workspaceRoots.length === 1 ? workspaceRoots[0] : null);
  if (!root || !isAbsoluteFilePath(root)) {
    return {
      error:
        "Cannot open relative file: no unambiguous source workspace root is available.",
    };
  }
  const separator = root.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/).filter((part) => part && part !== ".");
  if (!parts.length)
    return { error: "Cannot open an empty relative file path." };
  return {
    path: `${root.replace(/[\\/]+$/, "")}${separator}${parts.join(separator)}`,
  };
}
