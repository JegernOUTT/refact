import type { GitBranchesRoot } from "../../../../services/refact/gitRead";

export type BranchRefKind = "local" | "remote" | "tag";

export type BranchRef = {
  kind: BranchRefKind;
  /** Name accepted by the log `ref=` parameter. */
  name: string;
  label: string;
  current: boolean;
  ahead: number | null;
  behind: number | null;
  upstream: string | null;
  upstreamGone: boolean;
  tipOid: string | null;
};

export type BranchTreeNode =
  | { type: "head"; id: string; label: string; detail: string | null }
  | {
      type: "group";
      id: string;
      label: string;
      children: BranchTreeNode[];
    }
  | { type: "ref"; id: string; ref: BranchRef };

export type VisibleBranchRow = {
  node: BranchTreeNode;
  depth: number;
  parentId: string | null;
};

export const HEAD_NODE_ID = "head";
export const TAGS_GROUP_ID = "group:tags";

export const refNodeId = (kind: BranchRefKind, name: string) =>
  `ref:${kind}:${name}`;

const SHORT_OID_LENGTH = 7;

const compareNames = (left: string, right: string) =>
  left.localeCompare(right, undefined, { sensitivity: "base" }) ||
  left.localeCompare(right);

function refNode(ref: BranchRef): BranchTreeNode {
  return { type: "ref", id: refNodeId(ref.kind, ref.name), ref };
}

function emptyRef(
  kind: BranchRefKind,
  name: string,
  label: string,
  tipOid: string | null,
): BranchRef {
  return {
    kind,
    name,
    label,
    current: false,
    ahead: null,
    behind: null,
    upstream: null,
    upstreamGone: false,
    tipOid,
  };
}

function headDetail(data: GitBranchesRoot): string | null {
  if (data.head_detached) {
    return data.head_oid
      ? `detached at ${data.head_oid.slice(0, SHORT_OID_LENGTH)}`
      : "detached";
  }
  return data.current;
}

export function buildBranchTree(data: GitBranchesRoot): BranchTreeNode[] {
  const locals = [...data.branches]
    .sort((left, right) => compareNames(left.name, right.name))
    .map((branch) =>
      refNode({
        ...emptyRef("local", branch.name, branch.name, branch.tip_oid ?? null),
        current: branch.is_head,
        ahead: branch.ahead ?? null,
        behind: branch.behind ?? null,
        upstream: branch.upstream,
        upstreamGone: branch.upstream_gone ?? false,
      }),
    );

  const remoteNames = new Set(data.remotes ?? []);
  for (const branch of data.remote_branches ?? []) {
    remoteNames.add(branch.remote);
  }
  const remotes = [...remoteNames]
    .sort(compareNames)
    .map<BranchTreeNode>((remote) => ({
      type: "group",
      id: `group:remote:${remote}`,
      label: remote,
      children: (data.remote_branches ?? [])
        .filter((branch) => branch.remote === remote)
        .sort((left, right) => compareNames(left.short_name, right.short_name))
        .map((branch) =>
          refNode(
            emptyRef("remote", branch.name, branch.short_name, branch.tip_oid),
          ),
        ),
    }));

  const tags = [...(data.tags ?? [])]
    .sort((left, right) => compareNames(right.name, left.name))
    .map((tag) => refNode(emptyRef("tag", tag.name, tag.name, tag.target_oid)));

  const groups: BranchTreeNode[] = [
    { type: "head", id: HEAD_NODE_ID, label: "HEAD", detail: headDetail(data) },
    { type: "group", id: "group:local", label: "Local", children: locals },
  ];
  if (remotes.length > 0) {
    groups.push({
      type: "group",
      id: "group:remote",
      label: "Remote",
      children: remotes,
    });
  }
  if (tags.length > 0) {
    groups.push({
      type: "group",
      id: TAGS_GROUP_ID,
      label: "Tags",
      children: tags,
    });
  }
  return groups;
}

function filterNodes(nodes: BranchTreeNode[], query: string): BranchTreeNode[] {
  return nodes.flatMap<BranchTreeNode>((node) => {
    if (node.type === "head") return [];
    if (node.type === "ref") {
      return node.ref.name.toLowerCase().includes(query) ? [node] : [];
    }
    const children = filterNodes(node.children, query);
    return children.length > 0 ? [{ ...node, children }] : [];
  });
}

/** Search keeps matching refs and their groups and ignores collapsing. */
export function flattenBranchTree(
  nodes: BranchTreeNode[],
  collapsed: ReadonlySet<string>,
  search: string,
): VisibleBranchRow[] {
  const query = search.trim().toLowerCase();
  const source = query ? filterNodes(nodes, query) : nodes;
  const rows: VisibleBranchRow[] = [];
  const visit = (
    node: BranchTreeNode,
    depth: number,
    parentId: string | null,
  ) => {
    rows.push({ node, depth, parentId });
    if (node.type !== "group" || (!query && collapsed.has(node.id))) return;
    for (const child of node.children) visit(child, depth + 1, node.id);
  };
  for (const node of source) visit(node, 0, null);
  return rows;
}

export function allBranchRefs(nodes: BranchTreeNode[]): BranchRef[] {
  return nodes.flatMap((node) => {
    if (node.type === "ref") return [node.ref];
    if (node.type === "group") return allBranchRefs(node.children);
    return [];
  });
}
