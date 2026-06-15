import { useEffect, useRef } from "react";
import classNames from "classnames";
import {
  BookOpen,
  FileText,
  Link2,
  Repeat2,
  Search,
  Star,
  Target,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { Badge, Icon, Surface, VirtualList } from "../../components/ui";
import type { KnowledgeMemoRecord } from "../../services/refact/types";
import styles from "./MemoryListView.module.css";

interface MemoryListViewProps {
  memories: KnowledgeMemoRecord[];
  selectedId: string | null;
  onSelectId: (id: string) => void;
  linkedIds: Set<string>;
}

const KIND_CONFIG = {
  code: { icon: FileText },
  decision: { icon: Target },
  preference: { icon: Star },
  pattern: { icon: Repeat2 },
  lesson: { icon: BookOpen },
} as const;

type KindKey = keyof typeof KIND_CONFIG;
type KindConfig = {
  icon: LucideIcon;
};

function getKindConfig(kind: string | undefined): KindConfig {
  if (kind && kind in KIND_CONFIG) {
    return KIND_CONFIG[kind as KindKey];
  }
  return KIND_CONFIG.code;
}

function formatKind(kind: string): string {
  return kind.charAt(0).toUpperCase() + kind.slice(1);
}

export function MemoryListView({
  memories,
  selectedId,
  onSelectId,
  linkedIds,
}: MemoryListViewProps) {
  const cardRefs = useRef<Map<string, HTMLButtonElement>>(new Map());

  useEffect(() => {
    if (selectedId && cardRefs.current.has(selectedId)) {
      const element = cardRefs.current.get(selectedId);
      element?.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
      });
    }
  }, [selectedId]);

  if (memories.length === 0) {
    return (
      <Surface className={styles.emptyState} radius="none">
        <Icon icon={Search} size="lg" tone="faint" />
        <p className={styles.emptyText}>No memories to display</p>
      </Surface>
    );
  }

  return (
    <Surface className={styles.container} radius="none">
      <VirtualList
        className={classNames(styles.list, "rf-enter-rise")}
        height="100%"
        items={memories}
        getItemKey={(memory) => memory.memid}
        renderItem={(memory) => {
          const isSelected = selectedId === memory.memid;
          const isLinked = linkedIds.has(memory.memid);
          const kind = memory.kind ?? "code";
          const kindConfig = getKindConfig(memory.kind);

          return (
            <Surface
              className={classNames(
                styles.cardFrame,
                isSelected && styles.selected,
              )}
              key={memory.memid}
              variant={isSelected ? "selected" : "plain"}
              animated="rise"
            >
              <button
                ref={(el) => {
                  if (el) {
                    cardRefs.current.set(memory.memid, el);
                  } else {
                    cardRefs.current.delete(memory.memid);
                  }
                }}
                className={classNames(
                  styles.card,
                  "rf-pressable",
                  isSelected && styles.selected,
                )}
                onClick={() => onSelectId(memory.memid)}
                type="button"
                aria-pressed={isSelected}
              >
                <div className={styles.header}>
                  <div className={styles.headerLeft}>
                    <span
                      className={styles.kindBadge}
                      aria-label={`Kind: ${kind}`}
                    >
                      <Icon
                        className={styles.kindIcon}
                        icon={kindConfig.icon}
                        tone="muted"
                      />
                    </span>
                    <span className={styles.title}>
                      {memory.title ?? "Untitled"}
                    </span>
                  </div>
                  {isLinked ? (
                    <span
                      className={styles.linkBadge}
                      aria-label="Linked in graph"
                    >
                      <Icon
                        className={styles.linkIcon}
                        icon={Link2}
                        size="sm"
                        tone="muted"
                      />
                    </span>
                  ) : null}
                </div>

                <div className={styles.metadata}>
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>Kind:</span>
                    <span className={styles.metaValue}>{formatKind(kind)}</span>
                  </div>
                  {memory.tags.length > 0 ? (
                    <div className={styles.metaRow}>
                      <span className={styles.metaLabel}>Tags:</span>
                      <div className={styles.tags}>
                        {memory.tags.slice(0, 3).map((tag) => (
                          <Badge
                            key={tag}
                            className={styles.tagBadge}
                            tone="muted"
                            title={tag}
                          >
                            {tag}
                          </Badge>
                        ))}
                        {memory.tags.length > 3 ? (
                          <span className={styles.tagMore}>
                            +{memory.tags.length - 3}
                          </span>
                        ) : null}
                      </div>
                    </div>
                  ) : null}
                </div>
              </button>
            </Surface>
          );
        }}
      />
    </Surface>
  );
}
