import type { KnowledgeMemoRecord } from '../../services/refact/types';
import styles from './MemoryListView.module.css';

interface MemoryListViewProps {
  memories: KnowledgeMemoRecord[];
  selectedId: string | null;
  onSelectId: (id: string) => void;
  linkedIds: Set<string>;
}

const KIND_CONFIG = {
  code: { icon: '📄', color: '#3B82F6' },
  decision: { icon: '🎯', color: '#8B5CF6' },
  preference: { icon: '⭐', color: '#10B981' },
  pattern: { icon: '🔄', color: '#F59E0B' },
  lesson: { icon: '📚', color: '#06B6D4' },
} as const;

type KindKey = keyof typeof KIND_CONFIG;

function getKindConfig(kind: string | undefined): { icon: string; color: string } {
  if (kind && kind in KIND_CONFIG) {
    return KIND_CONFIG[kind as KindKey];
  }
  return KIND_CONFIG.code;
}

export function MemoryListView({
  memories,
  selectedId,
  onSelectId,
  linkedIds,
}: MemoryListViewProps) {
  if (memories.length === 0) {
    return (
      <div className={styles.emptyState}>
        <div className={styles.emptyIcon}>🔍</div>
        <p className={styles.emptyText}>No memories to display</p>
      </div>
    );
  }

  return (
    <div className={styles.container}>
      <div className={styles.grid}>
        {memories.map((memory) => {
          const isSelected = selectedId === memory.memid;
          const isLinked = linkedIds.has(memory.memid);
          const kind = memory.kind ?? 'code';
          const kindConfig = getKindConfig(memory.kind);

          return (
            <button
              key={memory.memid}
              className={`${styles.card} ${isSelected ? styles.selected : ''}`}
              onClick={() => onSelectId(memory.memid)}
              type="button"
              aria-pressed={isSelected}
            >
              <div className={styles.header}>
                <div className={styles.headerLeft}>
                  <span
                    className={styles.kindBadge}
                    style={{ backgroundColor: kindConfig.color }}
                    aria-label={`Kind: ${kind}`}
                  >
                    {kindConfig.icon}
                  </span>
                  <span className={styles.title}>
                    {memory.title ?? 'Untitled'}
                  </span>
                </div>
                {isLinked && (
                  <span className={styles.linkBadge} aria-label="Linked in graph">
                    🔗
                  </span>
                )}
              </div>

              <div className={styles.metadata}>
                <div className={styles.metaRow}>
                  <span className={styles.metaLabel}>Kind:</span>
                  <span className={styles.metaValue}>
                    {kind.charAt(0).toUpperCase() + kind.slice(1)}
                  </span>
                </div>
                {memory.tags.length > 0 && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>Tags:</span>
                    <div className={styles.tags}>
                      {memory.tags.slice(0, 3).map((tag) => (
                        <span key={tag} className={styles.tagDot} title={tag}>
                          ●
                        </span>
                      ))}
                      {memory.tags.length > 3 && (
                        <span className={styles.tagMore}>
                          +{memory.tags.length - 3}
                        </span>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
