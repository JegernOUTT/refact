import { useMemo, useState } from "react";

import { Chip, Select, Surface, Tabs } from "../../components/ui";
import { useGetKnowledgeGraphQuery } from "../../services/refact/knowledgeGraphApi";
import type { KnowledgeMemoRecord } from "../../services/refact/types";
import { KnowledgeGraphView } from "./KnowledgeGraphView";
import { MemoryDetailsEditor } from "./MemoryDetailsEditor";
import { MemoryListView } from "./MemoryListView";
import styles from "./KnowledgeWorkspace.module.css";

type KnowledgeTab = "memories" | "graph";
type MemorySortKey = "date" | "kind" | "title" | "tagCount";

const sortLabels: Record<MemorySortKey, string> = {
  date: "Updated/Created",
  kind: "Kind",
  title: "Title",
  tagCount: "Tag count",
};

function timestampValue(value: string | undefined): number {
  if (!value) return 0;
  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

function compareText(left: string | undefined, right: string | undefined) {
  return (left ?? "").localeCompare(right ?? "", undefined, {
    sensitivity: "base",
  });
}

function memoryMatchesTags(
  memory: KnowledgeMemoRecord,
  selectedTags: Set<string>,
) {
  if (selectedTags.size === 0) return true;
  return [...selectedTags].every((tag) => memory.tags.includes(tag));
}

function sortMemories(memories: KnowledgeMemoRecord[], sortKey: MemorySortKey) {
  return [...memories].sort((left, right) => {
    switch (sortKey) {
      case "date":
        return timestampValue(right.created) - timestampValue(left.created);
      case "kind":
        return (
          compareText(left.kind, right.kind) ||
          compareText(left.title, right.title)
        );
      case "title":
        return compareText(left.title, right.title);
      case "tagCount":
        return (
          right.tags.length - left.tags.length ||
          compareText(left.title, right.title)
        );
    }
  });
}

export function KnowledgeWorkspace() {
  const {
    data: graph,
    isLoading,
    error,
  } = useGetKnowledgeGraphQuery({ includeContent: true });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<KnowledgeTab>("memories");
  const [sortKey, setSortKey] = useState<MemorySortKey>("date");
  const [selectedTags, setSelectedTags] = useState<Set<string>>(
    () => new Set(),
  );

  const allDocNodes = useMemo(() => {
    if (!graph) return [];
    return graph.nodes.filter((node) => {
      const isDocNode =
        node.node_type === "doc" || node.node_type.startsWith("doc_");
      if (!isDocNode) return false;

      const kind = node.node_type.replace("doc_", "").toLowerCase();
      return (
        kind !== "deprecated" && kind !== "archived" && kind !== "trajectory"
      );
    });
  }, [graph]);

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

  const allTags = useMemo(() => {
    return [...new Set(memoryRecords.flatMap((memory) => memory.tags))].sort(
      (left, right) =>
        left.localeCompare(right, undefined, { sensitivity: "base" }),
    );
  }, [memoryRecords]);

  const filteredMemoryRecords = useMemo(() => {
    return sortMemories(
      memoryRecords.filter((memory) => memoryMatchesTags(memory, selectedTags)),
      sortKey,
    );
  }, [memoryRecords, selectedTags, sortKey]);

  const filteredDocNodes = useMemo(() => {
    if (selectedTags.size === 0) return allDocNodes;
    const filteredIds = new Set(
      filteredMemoryRecords.map((memory) => memory.memid),
    );
    return allDocNodes.filter((node) => filteredIds.has(node.id));
  }, [allDocNodes, filteredMemoryRecords, selectedTags]);

  const docDocEdges = useMemo(() => {
    if (!graph) return [];
    const docIds = new Set(filteredDocNodes.map((node) => node.id));
    return graph.edges.filter(
      (edge) => docIds.has(edge.source) && docIds.has(edge.target),
    );
  }, [graph, filteredDocNodes]);

  const linkedIds = useMemo(() => {
    const ids = new Set<string>();
    docDocEdges.forEach((edge) => {
      ids.add(edge.source);
      ids.add(edge.target);
    });
    return ids;
  }, [docDocEdges]);

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

  const activeTabIndex = activeTab === "memories" ? 0 : 1;

  const handleSelectMemory = (id: string | null) => {
    setSelectedId(id);
  };

  const handleMemoryDeleted = () => {
    setSelectedId(null);
  };

  const handleToggleTag = (tag: string) => {
    setSelectedTags((current) => {
      const next = new Set(current);
      if (next.has(tag)) {
        next.delete(tag);
      } else {
        next.add(tag);
      }
      return next;
    });
  };

  const handleClearTags = () => {
    setSelectedTags(new Set());
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
      <Tabs
        className={styles.tabs}
        value={activeTab}
        onValueChange={(value) => setActiveTab(value as KnowledgeTab)}
      >
        <Tabs.List
          activeIndex={activeTabIndex}
          className={styles.tabList}
          itemCount={2}
        >
          <Tabs.Trigger value="memories">Memories</Tabs.Trigger>
          <Tabs.Trigger value="graph">Graph</Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content className={styles.tabContent} value="memories">
          <div className={styles.memoriesPanel}>
            <Surface className={styles.listSection} variant="plain">
              <div className={styles.controls}>
                <div className={styles.sortControl}>
                  <span className={styles.controlLabel}>Sort</span>
                  <Select
                    value={sortKey}
                    onValueChange={(value) =>
                      setSortKey(value as MemorySortKey)
                    }
                  >
                    <Select.Trigger
                      aria-label="Sort memories"
                      className={styles.sortTrigger}
                    />
                    <Select.Content maxWidth="220px">
                      {Object.entries(sortLabels).map(([value, label]) => (
                        <Select.Item key={value} value={value}>
                          {label}
                        </Select.Item>
                      ))}
                    </Select.Content>
                  </Select>
                </div>

                {allTags.length > 0 ? (
                  <div
                    className={styles.tagFilters}
                    aria-label="Filter by tags"
                  >
                    <span className={styles.controlLabel}>Tags</span>
                    <div className={styles.tagList}>
                      {allTags.map((tag) => {
                        const selected = selectedTags.has(tag);
                        return (
                          <button
                            key={tag}
                            className={styles.tagButton}
                            type="button"
                            onClick={() => handleToggleTag(tag)}
                            aria-pressed={selected}
                          >
                            <Chip radius="chip" selected={selected}>
                              {tag}
                            </Chip>
                          </button>
                        );
                      })}
                      {selectedTags.size > 0 ? (
                        <button
                          className={styles.clearTagsButton}
                          type="button"
                          onClick={handleClearTags}
                        >
                          Clear
                        </button>
                      ) : null}
                    </div>
                  </div>
                ) : null}
              </div>

              <MemoryListView
                memories={filteredMemoryRecords}
                selectedId={selectedId}
                onSelectId={handleSelectMemory}
                linkedIds={linkedIds}
              />
            </Surface>

            <Surface className={styles.editorSection} variant="plain">
              <MemoryDetailsEditor
                memory={selectedMemory}
                onMemoryDeleted={handleMemoryDeleted}
              />
            </Surface>
          </div>
        </Tabs.Content>

        <Tabs.Content className={styles.tabContent} value="graph">
          <Surface className={styles.graphSection} variant="plain">
            <KnowledgeGraphView
              nodes={filteredDocNodes}
              edges={docDocEdges}
              selectedId={selectedId}
              onSelectId={handleSelectMemory}
              isLoading={isLoading}
              isActive={activeTab === "graph"}
            />
          </Surface>
        </Tabs.Content>
      </Tabs>
    </Surface>
  );
}
