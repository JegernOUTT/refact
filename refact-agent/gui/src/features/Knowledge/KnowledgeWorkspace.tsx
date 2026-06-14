import { useMemo, useState } from "react";
import { X } from "lucide-react";

import {
  Button,
  IconButton,
  LoadingState,
  Select,
  Sheet,
  Surface,
  Tabs,
} from "../../components/ui";
import {
  useGetKnowledgeGraphQuery,
  useRelinkMemoriesMutation,
} from "../../services/refact/knowledgeGraphApi";
import type {
  KnowledgeGraphNode,
  KnowledgeMemoRecord,
} from "../../services/refact/types";
import { KnowledgeGraphView } from "./KnowledgeGraphView";
import { isActiveKnowledgeDocNode } from "./knowledgeGraphFilters";
import { MemoryDetailsEditor } from "./MemoryDetailsEditor";
import { MemoryListView } from "./MemoryListView";
import { MemoryTagFilter } from "./MemoryTagFilter";
import styles from "./KnowledgeWorkspace.module.css";

type KnowledgeTab = "memories" | "graph";
type MemorySortKey = "date" | "kind" | "title" | "tagCount";

const sortLabels: Record<MemorySortKey, string> = {
  date: "Updated/Created",
  kind: "Kind",
  title: "Title",
  tagCount: "Tag count",
};

function nodeToRecord(node: KnowledgeGraphNode): KnowledgeMemoRecord {
  return {
    memid: node.id,
    tags: node.tags ?? [],
    content: node.content ?? "",
    title: node.title ?? node.label,
    kind: node.kind ?? node.node_type.replace("doc_", ""),
    file_path: node.file_path,
    created: node.created,
  };
}

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
  const [editingId, setEditingId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<KnowledgeTab>("memories");
  const [sortKey, setSortKey] = useState<MemorySortKey>("date");
  const [selectedTags, setSelectedTags] = useState<Set<string>>(
    () => new Set(),
  );
  const [relinkMemories, { isLoading: isRelinking }] =
    useRelinkMemoriesMutation();
  const [relinkResult, setRelinkResult] = useState<string | null>(null);

  const allDocNodes = useMemo(() => {
    if (!graph) return [];
    return graph.nodes.filter(isActiveKnowledgeDocNode);
  }, [graph]);

  const memoryRecords = useMemo(
    () => allDocNodes.map(nodeToRecord),
    [allDocNodes],
  );

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

  const linkedDocNodes = useMemo(
    () => filteredDocNodes.filter((node) => linkedIds.has(node.id)),
    [filteredDocNodes, linkedIds],
  );

  const editingMemory = useMemo((): KnowledgeMemoRecord | null => {
    if (!editingId) return null;
    const node = allDocNodes.find((n) => n.id === editingId);
    return node ? nodeToRecord(node) : null;
  }, [editingId, allDocNodes]);

  const activeTabIndex = activeTab === "memories" ? 0 : 1;

  const handleOpenMemory = (id: string) => {
    setSelectedId(id);
    setEditingId(id);
  };

  const handleSelectNode = (id: string | null) => {
    setSelectedId(id);
  };

  const handleCloseDetails = () => {
    setEditingId(null);
  };

  const handleMemoryDeleted = () => {
    setEditingId(null);
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

  const handleRelink = () => {
    setRelinkResult(null);
    void relinkMemories(undefined)
      .unwrap()
      .then((stats) => {
        setRelinkResult(
          `Linked ${stats.links_added} connections across ${stats.docs_updated} memories`,
        );
      })
      .catch(() => {
        setRelinkResult("Failed to rebuild links");
      });
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
          {isLoading ? (
            <LoadingState
              className={styles.loadingState}
              label="Loading memories..."
            />
          ) : (
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

                <MemoryTagFilter
                  allTags={allTags}
                  selectedTags={selectedTags}
                  onToggleTag={handleToggleTag}
                  onClearTags={handleClearTags}
                />
              </div>

              <MemoryListView
                memories={filteredMemoryRecords}
                selectedId={selectedId}
                onSelectId={handleOpenMemory}
                linkedIds={linkedIds}
              />
            </Surface>
          )}
        </Tabs.Content>

        <Tabs.Content className={styles.tabContent} value="graph">
          <Surface className={styles.graphSection} variant="plain">
            <div className={styles.graphToolbar}>
              <span className={styles.graphHint}>
                {relinkResult ?? `${linkedDocNodes.length} linked memories`}
              </span>
              <Button
                variant="soft"
                size="sm"
                onClick={handleRelink}
                loading={isRelinking}
              >
                Rebuild links
              </Button>
            </div>
            <div className={styles.graphBody}>
              <KnowledgeGraphView
                nodes={linkedDocNodes}
                edges={docDocEdges}
                selectedId={selectedId}
                onSelectId={handleSelectNode}
                isLoading={isLoading}
                isActive={activeTab === "graph"}
              />
            </div>
          </Surface>
        </Tabs.Content>
      </Tabs>

      <Sheet
        open={editingMemory !== null}
        onOpenChange={(open) => {
          if (!open) handleCloseDetails();
        }}
      >
        <Sheet.Content
          className={styles.detailsSheet}
          side="right"
          scrollable={false}
          maxWidth="560px"
        >
          <div className={styles.detailsHeader}>
            <Sheet.Title className={styles.detailsTitle}>
              {editingMemory?.title ?? "Memory"}
            </Sheet.Title>
            <Sheet.Close asChild>
              <IconButton
                icon={X}
                aria-label="Close memory details"
                variant="ghost"
                size="sm"
              />
            </Sheet.Close>
          </div>
          <div className={styles.detailsBody}>
            <MemoryDetailsEditor
              memory={editingMemory}
              onMemoryDeleted={handleMemoryDeleted}
            />
          </div>
        </Sheet.Content>
      </Sheet>
    </Surface>
  );
}
