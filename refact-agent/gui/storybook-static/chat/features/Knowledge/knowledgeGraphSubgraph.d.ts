import type { KnowledgeGraphNode, KnowledgeGraphEdge } from "../../services/refact/types";
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
export declare function makeEdgeId(source: string, target: string, edgeType: string): string;
export declare function buildSubgraph(params: SubgraphParams): SubgraphResult;
