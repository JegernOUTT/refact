import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import { Virtuoso } from "react-virtuoso";
import { useAppSelector } from "../../../../hooks/useAppSelector";
import { useDeleteTrajectoryMutation } from "../../../../services/refact/trajectories";
import type {
  StreamFilter,
  StreamGroup,
  StreamItem,
} from "../../streamSelectors";
import { selectStreamGroups } from "../../streamSelectors";
import { StreamRow } from "./StreamRow";
import { ThreadRail } from "./ThreadRail";
import { RowPeek } from "./RowPeek";
import styles from "./Stream.module.css";

const LIVE_LABEL = "Active now";

type FlatEntry =
  | { type: "header"; key: string; label: string }
  | { type: "row"; key: string; item: StreamItem; index: number };

function flatten(groups: StreamGroup[]): FlatEntry[] {
  const out: FlatEntry[] = [];
  for (const group of groups) {
    if (group.items.length === 0) continue;
    out.push({
      type: "header",
      key: `header-${group.label}`,
      label: group.label,
    });
    group.items.forEach((item, index) => {
      out.push({ type: "row", key: item.id, item, index });
    });
  }
  return out;
}

export type StreamSectionProps = {
  filter: StreamFilter;
  onOpenChat: (id: string) => void;
  onOpenTask: (id: string) => void;
};

export const StreamSection: React.FC<StreamSectionProps> = ({
  filter,
  onOpenChat,
  onOpenTask,
}) => {
  const [deleteTrajectory] = useDeleteTrajectoryMutation();
  const groups = useAppSelector((state) => selectStreamGroups(state, filter));

  const [expandedFamilies, setExpandedFamilies] = useState<Set<string>>(
    () => new Set(),
  );
  const [peekId, setPeekId] = useState<string | null>(null);

  const entries = useMemo(() => flatten(groups), [groups]);

  const handleToggleFamily = useCallback((id: string) => {
    setExpandedFamilies((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleTogglePeek = useCallback((id: string) => {
    setPeekId((prev) => (prev === id ? null : id));
  }, []);

  const handleOpen = useCallback(
    (item: StreamItem) => {
      if (item.kind === "task") onOpenTask(item.id);
      else onOpenChat(item.id);
    },
    [onOpenChat, onOpenTask],
  );

  const renderEntry = useCallback(
    (_index: number, entry: FlatEntry) => {
      if (entry.type === "header") {
        const isLive = entry.label === LIVE_LABEL;
        return (
          <div className={styles.groupHeader}>
            <span
              className={classNames(
                styles.groupLabel,
                isLive && styles.groupLabelLive,
              )}
            >
              {entry.label}
            </span>
            <span className={styles.groupRule} />
          </div>
        );
      }

      const { item, index } = entry;
      const familyOpen = expandedFamilies.has(item.id);
      const peekOpen = peekId === item.id;

      return (
        <div>
          <StreamRow
            item={item}
            index={index}
            isPeekOpen={peekOpen}
            isFamilyOpen={familyOpen}
            onOpen={handleOpen}
            onTogglePeek={handleTogglePeek}
            onToggleFamily={handleToggleFamily}
          />
          <div className={classNames(styles.xp, familyOpen && styles.open)}>
            <div className={styles.xpInner}>
              {familyOpen && (
                <ThreadRail rootId={item.id} onOpenChat={onOpenChat} />
              )}
            </div>
          </div>
          <div className={classNames(styles.xp, peekOpen && styles.open)}>
            <div className={styles.xpInner}>
              {peekOpen && (
                <RowPeek
                  item={item}
                  onOpen={() => handleOpen(item)}
                  onDelete={() => {
                    if (item.kind === "chat") {
                      void deleteTrajectory(item.id);
                    }
                    setPeekId(null);
                  }}
                />
              )}
            </div>
          </div>
        </div>
      );
    },
    [
      expandedFamilies,
      peekId,
      handleTogglePeek,
      handleToggleFamily,
      handleOpen,
      onOpenChat,
      deleteTrajectory,
    ],
  );

  return (
    <div className={styles.container} data-testid="stream-section">
      {entries.length === 0 ? (
        <div className={styles.empty}>Nothing here yet.</div>
      ) : (
        <Virtuoso
          data={entries}
          overscan={200}
          className={styles.list}
          computeItemKey={(_index, entry) => entry.key}
          itemContent={renderEntry}
        />
      )}
    </div>
  );
};
