import { useState, useMemo } from 'react';
import { useGetKnowledgeGraphQuery } from '../../services/refact/knowledgeGraphApi';
import { MemoryListView } from './MemoryListView';
import { KnowledgeGraphView } from './KnowledgeGraphView';
import { MemoryDetailsEditor } from './MemoryDetailsEditor';
import type { KnowledgeMemoRecord } from '../../services/refact/types';
import styles from './KnowledgeWorkspace.module.css';

export function KnowledgeWorkspace() {
  const { data: graph, isLoading, error } = useGetKnowledgeGraphQuery(undefined);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const allDocNodes = useMemo(() => {
    if (!graph) return [];
    return graph.nodes.filter((node) => {
      const isDocNode = node.node_type === 'doc' || node.node_type.startsWith('doc_');
      if (!isDocNode) return false;

      const kind = node.node_type.replace('doc_', '').toLowerCase();
      return kind !== 'deprecated' &&
             kind !== 'archived' &&
             kind !== 'trajectory';
    });
  }, [graph]);

  const docDocEdges = useMemo(() => {
    if (!graph) return [];
    const docIds = new Set(allDocNodes.map((n) => n.id));
    return graph.edges.filter(
      (edge) => docIds.has(edge.source) && docIds.has(edge.target)
    );
  }, [graph, allDocNodes]);

  const linkedIds = useMemo(() => {
    const ids = new Set<string>();
    docDocEdges.forEach((e) => {
      ids.add(e.source);
      ids.add(e.target);
    });
    return ids;
  }, [docDocEdges]);

  const linkedDocNodes = useMemo(
    () => allDocNodes.filter((n) => linkedIds.has(n.id)),
    [allDocNodes, linkedIds]
  );

  const memoryRecords = useMemo((): KnowledgeMemoRecord[] => {
    return allDocNodes.map((node) => ({
      memid: node.id,
      tags: node.tags ?? [],
      content: node.content ?? '',
      title: node.title ?? node.label,
      kind: node.kind ?? node.node_type.replace('doc_', ''),
      file_path: node.file_path,
      created: node.created,
    }));
  }, [allDocNodes]);

  const selectedMemory = useMemo((): KnowledgeMemoRecord | null => {
    if (!selectedId) return null;
    const node = allDocNodes.find((n) => n.id === selectedId);
    if (!node) return null;
    return {
      memid: node.id,
      tags: node.tags ?? [],
      content: node.content ?? '',
      title: node.title ?? node.label,
      kind: node.kind ?? node.node_type.replace('doc_', ''),
      file_path: node.file_path,
      created: node.created,
    };
  }, [selectedId, allDocNodes]);

  const handleSelectMemory = (id: string | null) => {
    setSelectedId(id);
  };

  const handleMemoryDeleted = () => {
    setSelectedId(null);
  };

  if (error) {
    return (
      <div className={styles.workspace}>
        <div className={styles.error}>
          <p>Failed to load knowledge graph</p>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.workspace}>
      <div className={styles.editorSection}>
        <MemoryDetailsEditor
          memory={selectedMemory}
          onMemoryDeleted={handleMemoryDeleted}
        />
      </div>

      <div className={styles.listSection}>
        <MemoryListView
          memories={memoryRecords}
          selectedId={selectedId}
          onSelectId={handleSelectMemory}
          linkedIds={linkedIds}
        />
      </div>

      <div className={styles.graphSection}>
        <KnowledgeGraphView
          nodes={linkedDocNodes}
          edges={docDocEdges}
          selectedId={selectedId}
          onSelectId={handleSelectMemory}
          isLoading={isLoading}
        />
      </div>
    </div>
  );
}
