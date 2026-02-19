import type {
  KnowledgeGraphNode,
  KnowledgeGraphEdge,
} from "../../services/refact/types";

export type SubgraphParams = {
  seedId: string;
  depth: 1 | 2;
  nodes: KnowledgeGraphNode[];
  edges: KnowledgeGraphEdge[];
  includeNode: (node: KnowledgeGraphNode) => boolean;
};

export type SubgraphResult = {
  nodeIds: Set<string>;
  edgeIds: Set<string>;
};

export function makeEdgeId(
  source: string,
  target: string,
  edgeType: string,
): string {
  return JSON.stringify([source, target, edgeType]);
}

export function buildSubgraph(params: SubgraphParams): SubgraphResult {
  const { seedId, depth, nodes, edges, includeNode } = params;

  const nodeIndex = new Map(nodes.map((n) => [n.id, n]));
  const seedNode = nodeIndex.get(seedId);

  if (!seedNode) {
    return { nodeIds: new Set(), edgeIds: new Set() };
  }

  const nodeIds = new Set<string>();
  const queue: { id: string; d: number }[] = [{ id: seedId, d: 0 }];

  while (queue.length > 0) {
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const { id, d } = queue.shift()!;

    if (nodeIds.has(id)) continue;

    const node = nodeIndex.get(id);
    if (!node || !includeNode(node)) continue;

    nodeIds.add(id);

    if (d < depth) {
      for (const edge of edges) {
        if (edge.source === id && !nodeIds.has(edge.target)) {
          queue.push({ id: edge.target, d: d + 1 });
        }
        if (edge.target === id && !nodeIds.has(edge.source)) {
          queue.push({ id: edge.source, d: d + 1 });
        }
      }
    }
  }

  const edgeIds = new Set<string>();
  for (const edge of edges) {
    if (nodeIds.has(edge.source) && nodeIds.has(edge.target)) {
      edgeIds.add(makeEdgeId(edge.source, edge.target, edge.edge_type));
    }
  }

  return { nodeIds, edgeIds };
}
