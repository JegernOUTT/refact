import type {
  FilesTreeEntry,
  FilesSort,
  FilesDirection,
} from "../../../services/refact/files";
import type { PrivacyPolicy } from "../../../services/refact/privacy";

export type VisibleTreeEntry = FilesTreeEntry & {
  depth: number;
};

export type TreeChildrenByPath = Record<string, FilesTreeEntry[] | undefined>;

export const flattenVisibleTree = (
  rootEntries: FilesTreeEntry[],
  expandedDirectories: ReadonlySet<string>,
  childrenByPath: TreeChildrenByPath,
  showIgnored = false,
  sort: FilesSort = "name",
  direction: FilesDirection = "asc",
): VisibleTreeEntry[] => {
  const visible: VisibleTreeEntry[] = [];

  const visit = (entries: FilesTreeEntry[], depth: number) => {
    for (const entry of sortTreeEntries(entries, sort, direction)) {
      if (entry.ignored && !showIgnored) continue;
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
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/u, "");
  if (
    !normalized ||
    /^[A-Za-z]:$/u.test(normalized) ||
    /^\/\/[^/]+\/[^/]+$/u.test(normalized)
  )
    return null;
  const index = normalized.lastIndexOf("/");
  if (index < 0) return null;
  if (index === 0) return "/";
  const parent = normalized.slice(0, index);
  return /^[A-Za-z]:$/u.test(parent) ? `${parent}/` : parent;
};

export const pathBasename = (path: string): string => {
  const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
  return normalized.slice(normalized.lastIndexOf("/") + 1) || normalized;
};

const normalizePrivacyValue = (value: string): string =>
  value.normalize("NFC").toLowerCase();

export const movePathToPrivacyZone = (
  policy: PrivacyPolicy,
  path: string,
  zoneName: string,
): PrivacyPolicy => {
  const normalizedPath = normalizePrivacyValue(path);
  const targetIndex = policy.zones.findIndex((zone) => zone.name === zoneName);
  if (targetIndex === -1) return policy;

  return {
    ...policy,
    blocked: policy.blocked.filter(
      (pattern) => normalizePrivacyValue(pattern) !== normalizedPath,
    ),
    zones: policy.zones.map((zone, index) => {
      const patterns = zone.patterns.filter(
        (pattern) => normalizePrivacyValue(pattern) !== normalizedPath,
      );
      if (index !== targetIndex) return { ...zone, patterns };
      return { ...zone, patterns: [path, ...patterns] };
    }),
  };
};

export const sortTreeEntries = (
  entries: FilesTreeEntry[],
  sort: FilesSort,
  direction: FilesDirection,
): FilesTreeEntry[] => {
  const compare = (a: string, b: string) =>
    a.toLowerCase() < b.toLowerCase()
      ? -1
      : a.toLowerCase() > b.toLowerCase()
        ? 1
        : 0;
  const extension = (name: string) =>
    name.lastIndexOf(".") > 0 ? name.slice(name.lastIndexOf(".") + 1) : "";
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    if (
      sort === "size" &&
      (a.size === null || b.size === null) &&
      a.size !== b.size
    )
      return a.size === null ? 1 : -1;
    const primary =
      sort === "size"
        ? (a.size ?? 0) - (b.size ?? 0)
        : sort === "type"
          ? compare(extension(a.name), extension(b.name))
          : compare(a.name, b.name);
    return (
      (direction === "asc" ? primary : -primary) ||
      (a.path < b.path ? -1 : a.path > b.path ? 1 : 0)
    );
  });
};
