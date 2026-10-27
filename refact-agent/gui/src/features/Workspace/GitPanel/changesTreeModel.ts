import type {
  GitFileStatus,
  NormalizedGitStatus,
} from "../../../services/refact/gitRead";

export type ChangesGroupKind = "changes" | "conflicts" | "unversioned";

export type ChangesEntryKind = "change" | "untracked" | "conflict";

export type ChangesTreeEntry = {
  id: string;
  kind: ChangesEntryKind;
  path: string;
  absolutePath: string;
  oldPath: string | null;
  status: GitFileStatus | null;
  name: string;
  directory: string;
};

export type ChangesGroupNode = {
  type: "group";
  id: string;
  group: ChangesGroupKind;
  label: string;
  fileCount: number;
  children: ChangesTreeNode[];
};

export type ChangesDirNode = {
  type: "dir";
  id: string;
  group: ChangesGroupKind;
  path: string;
  label: string;
  fileCount: number;
  children: ChangesTreeNode[];
};

export type ChangesFileNode = {
  type: "file";
  id: string;
  group: ChangesGroupKind;
  entry: ChangesTreeEntry;
};

export type ChangesTreeNode =
  | ChangesGroupNode
  | ChangesDirNode
  | ChangesFileNode;
export type ChangesContainerNode = ChangesGroupNode | ChangesDirNode;

export type VisibleChangesRow = {
  node: ChangesTreeNode;
  depth: number;
  parentId: string | null;
};

export type InclusionState = {
  excludedChanges: ReadonlySet<string>;
  includedUnversioned: ReadonlySet<string>;
};

export type CheckState = boolean | "indeterminate";

const GROUP_ORDER: ChangesGroupKind[] = ["changes", "conflicts", "unversioned"];

const GROUP_LABELS: Record<ChangesGroupKind, string> = {
  changes: "Changes",
  conflicts: "Merge Conflicts",
  unversioned: "Unversioned Files",
};

const GROUP_ENTRY_KIND: Record<ChangesGroupKind, ChangesEntryKind> = {
  changes: "change",
  conflicts: "conflict",
  unversioned: "untracked",
};

export const groupNodeId = (group: ChangesGroupKind) => `group:${group}`;

export const fileNodeId = (kind: ChangesEntryKind, path: string) =>
  `${kind}:${path}`;

const dirNodeId = (group: ChangesGroupKind, path: string) =>
  `dir:${group}:${path}`;

function splitPath(path: string): { name: string; directory: string } {
  const index = path.lastIndexOf("/");
  return index === -1
    ? { name: path, directory: "" }
    : { name: path.slice(index + 1), directory: path.slice(0, index) };
}

export function makeEntry(
  kind: ChangesEntryKind,
  path: string,
  absolutePath: string,
  status: GitFileStatus | null,
  oldPath: string | null,
): ChangesTreeEntry {
  return {
    id: fileNodeId(kind, path),
    kind,
    path,
    absolutePath,
    oldPath,
    status,
    ...splitPath(path),
  };
}

export function entriesFromStatus(
  status: NormalizedGitStatus,
): Record<ChangesGroupKind, ChangesTreeEntry[]> {
  return {
    changes: status.changes.map((change) =>
      makeEntry(
        "change",
        change.relative_path,
        change.absolute_path,
        change.status,
        change.old_path,
      ),
    ),
    conflicts: status.conflicted.map((file) =>
      makeEntry("conflict", file.relative_path, file.absolute_path, null, null),
    ),
    unversioned: status.untracked.map((file) =>
      makeEntry(
        "untracked",
        file.relative_path,
        file.absolute_path,
        null,
        null,
      ),
    ),
  };
}

const compareText = (left: string, right: string) =>
  left.localeCompare(right, undefined, { sensitivity: "base" }) ||
  left.localeCompare(right);

type DirBuilder = {
  path: string;
  dirs: Map<string, DirBuilder>;
  files: ChangesTreeEntry[];
};

function countFiles(nodes: ChangesTreeNode[]): number {
  return nodes.reduce(
    (count, node) => count + (node.type === "file" ? 1 : node.fileCount),
    0,
  );
}

function buildDirChildren(
  group: ChangesGroupKind,
  builder: DirBuilder,
): ChangesTreeNode[] {
  const dirs = [...builder.dirs.values()]
    .map((child) => buildDirNode(group, child, builder.path))
    .sort((left, right) => compareText(left.label, right.label));
  const files = [...builder.files]
    .sort((left, right) => compareText(left.name, right.name))
    .map<ChangesFileNode>((entry) => ({
      type: "file",
      id: entry.id,
      group,
      entry,
    }));
  return [...dirs, ...files];
}

function buildDirNode(
  group: ChangesGroupKind,
  builder: DirBuilder,
  parentPath: string,
): ChangesDirNode {
  let current = builder;
  while (current.files.length === 0 && current.dirs.size === 1) {
    const [only] = current.dirs.values();
    current = only;
  }
  const label = parentPath
    ? current.path.slice(parentPath.length + 1)
    : current.path;
  const children = buildDirChildren(group, current);
  return {
    type: "dir",
    id: dirNodeId(group, current.path),
    group,
    path: current.path,
    label,
    fileCount: countFiles(children),
    children,
  };
}

export function groupChildren(
  group: ChangesGroupKind,
  entries: ChangesTreeEntry[],
  groupByDirectory: boolean,
): ChangesTreeNode[] {
  if (!groupByDirectory) {
    return [...entries]
      .sort(
        (left, right) =>
          compareText(left.name, right.name) ||
          compareText(left.directory, right.directory),
      )
      .map((entry) => ({ type: "file", id: entry.id, group, entry }));
  }
  const root: DirBuilder = { path: "", dirs: new Map(), files: [] };
  for (const entry of entries) {
    let cursor = root;
    if (entry.directory) {
      for (const segment of entry.directory.split("/")) {
        const path = cursor.path ? `${cursor.path}/${segment}` : segment;
        let next = cursor.dirs.get(segment);
        if (!next) {
          next = { path, dirs: new Map(), files: [] };
          cursor.dirs.set(segment, next);
        }
        cursor = next;
      }
    }
    cursor.files.push(entry);
  }
  return buildDirChildren(group, root);
}

export function buildChangesTree(
  status: NormalizedGitStatus,
  groupByDirectory: boolean,
): ChangesGroupNode[] {
  const entries = entriesFromStatus(status);
  return GROUP_ORDER.filter(
    (group) => group === "changes" || entries[group].length > 0,
  ).map((group) => {
    const children = groupChildren(group, entries[group], groupByDirectory);
    return {
      type: "group",
      id: groupNodeId(group),
      group,
      label: GROUP_LABELS[group],
      fileCount: countFiles(children),
      children,
    };
  });
}

export function flattenChangesTree(
  groups: ChangesTreeNode[],
  collapsed: ReadonlySet<string>,
): VisibleChangesRow[] {
  const rows: VisibleChangesRow[] = [];
  const visit = (
    node: ChangesTreeNode,
    depth: number,
    parentId: string | null,
  ) => {
    rows.push({ node, depth, parentId });
    if (node.type === "file" || collapsed.has(node.id)) return;
    for (const child of node.children) visit(child, depth + 1, node.id);
  };
  for (const node of groups) visit(node, 0, null);
  return rows;
}

export function containerIds(groups: ChangesGroupNode[]): string[] {
  const ids: string[] = [];
  const visit = (node: ChangesTreeNode) => {
    if (node.type === "file") return;
    ids.push(node.id);
    node.children.forEach(visit);
  };
  groups.forEach(visit);
  return ids;
}

export function collectEntries(node: ChangesTreeNode): ChangesTreeEntry[] {
  if (node.type === "file") return [node.entry];
  return node.children.flatMap(collectEntries);
}

export function isCheckable(node: ChangesTreeNode | ChangesTreeEntry): boolean {
  if ("kind" in node) return node.kind !== "conflict";
  return node.group !== "conflicts";
}

export function isEntryIncluded(
  entry: ChangesTreeEntry,
  inclusion: InclusionState,
): boolean {
  if (entry.kind === "change")
    return !inclusion.excludedChanges.has(entry.path);
  if (entry.kind === "untracked") {
    return inclusion.includedUnversioned.has(entry.path);
  }
  return false;
}

export function nodeCheckState(
  node: ChangesTreeNode,
  inclusion: InclusionState,
): CheckState {
  const entries = collectEntries(node).filter(isCheckable);
  if (entries.length === 0) return false;
  const included = entries.filter((entry) =>
    isEntryIncluded(entry, inclusion),
  ).length;
  if (included === 0) return false;
  return included === entries.length ? true : "indeterminate";
}

export function applyInclusion(
  inclusion: InclusionState,
  entries: ChangesTreeEntry[],
  include: boolean,
): { excludedChanges: string[]; includedUnversioned: string[] } {
  const excluded = new Set(inclusion.excludedChanges);
  const unversioned = new Set(inclusion.includedUnversioned);
  for (const entry of entries) {
    if (entry.kind === "change") {
      if (include) excluded.delete(entry.path);
      else excluded.add(entry.path);
    } else if (entry.kind === "untracked") {
      if (include) unversioned.add(entry.path);
      else unversioned.delete(entry.path);
    }
  }
  return {
    excludedChanges: [...excluded].sort(),
    includedUnversioned: [...unversioned].sort(),
  };
}

export function pruneInclusion(
  inclusion: InclusionState,
  status: NormalizedGitStatus,
): { excludedChanges: string[]; includedUnversioned: string[] } {
  const changePaths = new Set(
    status.changes.map((change) => change.relative_path),
  );
  const untrackedPaths = new Set(
    status.untracked.map((file) => file.relative_path),
  );
  return {
    excludedChanges: [...inclusion.excludedChanges]
      .filter((path) => changePaths.has(path))
      .sort(),
    includedUnversioned: [...inclusion.includedUnversioned]
      .filter((path) => untrackedPaths.has(path))
      .sort(),
  };
}

export function includedEntries(
  status: NormalizedGitStatus,
  inclusion: InclusionState,
): ChangesTreeEntry[] {
  const entries = entriesFromStatus(status);
  return [...entries.changes, ...entries.unversioned].filter((entry) =>
    isEntryIncluded(entry, inclusion),
  );
}

export function commitPaths(entries: ChangesTreeEntry[]): string[] {
  const paths = new Set<string>();
  for (const entry of entries) {
    paths.add(entry.path);
    if (entry.status === "RENAMED" && entry.oldPath) paths.add(entry.oldPath);
  }
  return [...paths];
}

export function checkableCount(status: NormalizedGitStatus): number {
  return status.changes.length + status.untracked.length;
}

export function entryKindForGroup(group: ChangesGroupKind): ChangesEntryKind {
  return GROUP_ENTRY_KIND[group];
}

export function parentRowIndex(
  rows: VisibleChangesRow[],
  index: number,
): number {
  const parentId = rows[index]?.parentId;
  if (!parentId) return -1;
  return rows.findIndex((row) => row.node.id === parentId);
}
