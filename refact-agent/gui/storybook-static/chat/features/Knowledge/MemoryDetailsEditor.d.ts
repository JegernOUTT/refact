import { JSX } from 'react/jsx-runtime';
import type { KnowledgeMemoRecord } from "../../services/refact/types";
interface MemoryDetailsEditorProps {
    memory: KnowledgeMemoRecord | null;
    onMemoryUpdated?: () => void;
    onMemoryDeleted?: () => void;
}
export declare function MemoryDetailsEditor({ memory, onMemoryUpdated, onMemoryDeleted, }: MemoryDetailsEditorProps): JSX.Element;
export {};
