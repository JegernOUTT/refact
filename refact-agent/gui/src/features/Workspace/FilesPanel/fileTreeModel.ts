import type { FilesTreeEntry } from "../../../services/refact/files";

export type VisibleTreeEntry = FilesTreeEntry & {
  depth: number;
};

export type TreeChildrenByPath = Record<string, FilesTreeEntry[] | undefined>;

export const flattenVisibleTree = (
  rootEntries: FilesTreeEntry[],
  expandedDirectories: ReadonlySet<string>,
  childrenByPath: TreeChildrenByPath,
): VisibleTreeEntry[] => {
  const visible: VisibleTreeEntry[] = [];

  const visit = (entries: FilesTreeEntry[], depth: number) => {
    for (const entry of entries) {
      visible.push({ ...entry, depth });
      if (entry.kind === "dir" && expandedDirectories.has(entry.path)) {
        const children = childrenByPath[entry.path];
        if (children) visit(children, depth + 1);
      }
    }
  };

  visit(rootEntries, 0);
  return visible;
};

export const parentDirectoryPath = (path: string): string | null => {
  const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
  const index = normalized.lastIndexOf("/");
  if (index < 0) return null;
  if (index === 0) return "/";
  return normalized.slice(0, index);
};

export const pathBasename = (path: string): string => {
  const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
  return normalized.slice(normalized.lastIndexOf("/") + 1) || normalized;
};
