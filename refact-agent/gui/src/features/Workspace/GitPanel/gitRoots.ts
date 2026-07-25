function comparableRoot(root: string): string {
  const normalized = root.replace(/\\/g, "/");
  return normalized.length > 1 ? normalized.replace(/\/+$/, "") : normalized;
}

export function workspaceRootForGitRoot(
  configuredRoots: string[],
  gitRoot: string,
  explicitSourceRoot?: string | null,
): string {
  if (explicitSourceRoot) return explicitSourceRoot;

  const comparableGitRoot = comparableRoot(gitRoot);
  const candidates = configuredRoots
    .map((root) => ({ root, comparable: comparableRoot(root) }))
    .sort(
      (left, right) =>
        right.comparable.length - left.comparable.length ||
        left.comparable.localeCompare(right.comparable) ||
        left.root.localeCompare(right.root),
    );
  const exact = candidates.find(
    (candidate) => candidate.comparable === comparableGitRoot,
  );
  if (exact) return exact.root;

  const enclosing = candidates.find((candidate) =>
    comparableGitRoot.startsWith(`${candidate.comparable}/`),
  );
  if (enclosing) return enclosing.root;

  const enclosed = candidates.filter((candidate) =>
    candidate.comparable.startsWith(`${comparableGitRoot}/`),
  );
  return enclosed.length === 1 ? enclosed[0]?.root ?? gitRoot : gitRoot;
}
