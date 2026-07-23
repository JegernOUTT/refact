function comparableRoot(root: string): string {
  const normalized = root.replace(/\\/g, "/");
  return normalized.length > 1 ? normalized.replace(/\/+$/, "") : normalized;
}

export function workspaceRootForGitRoot(
  configuredRoots: string[],
  gitRoot: string,
): string {
  const comparableGitRoot = comparableRoot(gitRoot);
  return (
    configuredRoots.find((root) => {
      const comparableWorkspaceRoot = comparableRoot(root);
      return (
        comparableWorkspaceRoot === comparableGitRoot ||
        comparableWorkspaceRoot.startsWith(`${comparableGitRoot}/`) ||
        comparableGitRoot.startsWith(`${comparableWorkspaceRoot}/`)
      );
    }) ?? gitRoot
  );
}
