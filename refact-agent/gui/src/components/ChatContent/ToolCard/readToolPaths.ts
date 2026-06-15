export interface ReadToolArgs {
  paths?: unknown;
  path?: unknown;
}

function splitPathList(value: string): string[] {
  return value
    .split(",")
    .map((path) => path.trim())
    .filter(Boolean);
}

export function normalizeReadPaths(args: ReadToolArgs): string[] {
  const value = args.paths ?? args.path;

  if (typeof value === "string") return splitPathList(value);

  if (Array.isArray(value)) {
    return value.flatMap((path) =>
      typeof path === "string" ? splitPathList(path) : [],
    );
  }

  return [];
}
