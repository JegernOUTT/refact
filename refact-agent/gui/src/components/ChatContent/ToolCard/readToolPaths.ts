import type { ChatContextFile } from "../../../services/refact/types";

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

export function contextFileLineCount(file: ChatContextFile): number {
  return file.file_content ? file.file_content.split("\n").length : 0;
}

export function filesTarget(paths: string[], fallback: string): string {
  if (paths.length === 0) return fallback;
  const first = paths[0].split("/").pop() ?? paths[0];
  return paths.length > 1 ? `${first} +${paths.length - 1}` : first;
}
