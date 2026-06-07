import { JSX } from 'react/jsx-runtime';
import type { KnowledgeMemoRecord } from "../../services/refact/types";
interface MemoryListViewProps {
    memories: KnowledgeMemoRecord[];
    selectedId: string | null;
    onSelectId: (id: string) => void;
    linkedIds: Set<string>;
}
export declare function MemoryListView({ memories, selectedId, onSelectId, linkedIds, }: MemoryListViewProps): JSX.Element;
export {};
