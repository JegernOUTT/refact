import { useMemo, useState } from "react";

import { Surface } from "../../components/ui";
import { useGetKnowledgeGraphQuery } from "../../services/refact/knowledgeGraphApi";
import type { KnowledgeMemoRecord } from "../../services/refact/types";
import { KnowledgeGraphView } from "./KnowledgeGraphView";
import { MemoryDetailsEditor } from "./MemoryDetailsEditor";
import { MemoryListView } from "./MemoryListView";
import styles from "./KnowledgeWorkspace.module.css";

export function KnowledgeWorkspace() {
  const {
    data: graph,
    isLoading,
    error,
  } = useGetKnowledgeGraphQuery({ includeContent: true });
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const allDocNodes = useMemo(() => {
    if (!graph) return [];
    return graph.nodes.filter((node) => {
      const isDocNode = node.node_type === "doc" || node.node_type.startsWith("doc_");
      if (!isDocNode) return false;

      const kind = node.node_type.replace("doc_", "").toLowerCase();
      return kind !== "deprecated" && kind !== "archived" && kind !== "trajectory";
    });
  }, [graph]);

  const docDocEdges = useMemo(() => {
    if (!graph) return [];
    const docIds = new Set(allDocNodes.map((n) => n.id));
    return graph.edges.filter((edge) => docIds.has(edge.source) && docIds.has(edge.target));
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
    [allDocNodes, linkedIds],
  );

  const memoryRecords = useMemo((): KnowledgeMemoRecord[] => {
    return allDocNodes.map((node) => ({
      memid: node.id,
      tags: node.tags ?? [],
      content: node.content ?? "",
      title: node.title ?? node.label,
      kind: node.kind ?? node.node_type.replace("doc_", ""),
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
      content: node.content ?? "",
      title: node.title ?? node.label,
      kind: node.kind ?? node.node_type.replace("doc_", ""),
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
      <Surface className={styles.workspace} radius="none">
        <Surface className={styles.error} variant="plain">
          <p>Failed to load knowledge graph</p>
        </Surface>
      </Surface>
    );
  }

  return (
    <Surface className={styles.workspace} radius="none">
      <Surface className={styles.editorSection} variant="plain">
        <MemoryDetailsEditor memory={selectedMemory} onMemoryDeleted={handleMemoryDeleted} />
      </Surface>

      <Surface className={styles.listSection} variant="plain">
        <MemoryListView
          memories={memoryRecords}
          selectedId={selectedId}
          onSelectId={handleSelectMemory}
          linkedIds={linkedIds}
        />
      </Surface>

      <Surface className={styles.graphSection} variant="plain">
        <KnowledgeGraphView
          nodes={linkedDocNodes}
          edges={docDocEdges}
          selectedId={selectedId}
          onSelectId={handleSelectMemory}
          isLoading={isLoading}
        />
      </Surface>
    </Surface>
  );
}
