import { JSX } from 'react/jsx-runtime';
import type { KnowledgeGraphNode, KnowledgeGraphEdge } from "../../services/refact/types";
interface KnowledgeGraphViewProps {
    nodes: KnowledgeGraphNode[];
    edges: KnowledgeGraphEdge[];
    selectedId: string | null;
    onSelectId: (id: string | null) => void;
    isLoading?: boolean;
}
export declare function KnowledgeGraphView({ nodes, edges, selectedId, onSelectId, isLoading, }: KnowledgeGraphViewProps): JSX.Element;
export {};
