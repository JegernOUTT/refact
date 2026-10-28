import type { DiffChunk } from "../../../services/refact/types";
import { basename } from "./utils";

function countNonEmptyLines(text: string): number {
  let count = 0;
  let hasContent = false;

  for (const char of text) {
    if (char === "\n") {
      if (hasContent) count++;
      hasContent = false;
    } else if (char !== "\r" && char !== " " && char !== "\t") {
      hasContent = true;
    }
  }

  return hasContent ? count + 1 : count;
}

export function getDiffStats(diffs: DiffChunk[]): {
  added: number;
  removed: number;
} {
  let added = 0;
  let removed = 0;
  for (const diff of diffs) {
    added += countNonEmptyLines(diff.lines_add);
    removed += countNonEmptyLines(diff.lines_remove);
  }
  return { added, removed };
}

export function fileListTarget(paths: string[]): string {
  if (paths.length === 0) return "file";
  const first = basename(paths[0]);
  return paths.length > 1 ? `${first} +${paths.length - 1}` : first;
}

export function diffStatsMeta(diffs: DiffChunk[]): string | undefined {
  if (diffs.length === 0) return undefined;
  const { added, removed } = getDiffStats(diffs);
  return `+${added} −${removed}`;
}

const PATCH_FILE_PATTERN =
  /^(?:\*\*\* (?:Update|Add|Delete) File: |\+\+\+ (?:b\/)?)(.+)$/gm;

export function editToolFilePaths(rawArgs: string): string[] {
  try {
    const args = JSON.parse(rawArgs) as Record<string, unknown>;
    if (typeof args.path === "string") return [args.path];
    if (typeof args.patch !== "string") return [];
    const paths = Array.from(args.patch.matchAll(PATCH_FILE_PATTERN), (match) =>
      match[1].trim(),
    ).filter((path) => path.length > 0 && path !== "/dev/null");
    return Array.from(new Set(paths));
  } catch {
    return [];
  }
}
