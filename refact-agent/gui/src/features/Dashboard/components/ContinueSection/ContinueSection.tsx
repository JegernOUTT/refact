import React from "react";
import { ListChecks, MessageSquare } from "lucide-react";
import { Icon } from "../../../../components/ui";
import { useAppSelector } from "../../../../hooks";
import { formatRelativeTime } from "../../dateUtils";
import {
  isActiveStatus,
  selectContinueItems,
  type StreamItem,
} from "../../streamSelectors";
import { formatCompactNumber } from "../Stream/streamRowUtils";
import styles from "./ContinueSection.module.css";

type ContinueSectionProps = {
  onOpenChat: (id: string) => void;
  onOpenTask: (id: string) => void;
};

function isLive(item: StreamItem): boolean {
  return isActiveStatus(item.status) || (item.agentsActive ?? 0) > 0;
}

function liveLabel(item: StreamItem): string {
  const agents = item.agentsActive ?? 0;
  if (agents > 0) return `${agents} agent${agents === 1 ? "" : "s"}`;
  return "streaming";
}

function chatSubLine(item: StreamItem): string | null {
  const parts: string[] = [];
  if (typeof item.messageCount === "number" && item.messageCount > 0) {
    parts.push(`${item.messageCount} msgs`);
  }
  if (item.mode) parts.push(item.mode);
  return parts.length > 0 ? parts.join(" · ") : null;
}

const ContinueCard: React.FC<{
  item: StreamItem;
  onOpen: () => void;
}> = ({ item, onOpen }) => {
  const live = isLive(item);
  const total = item.cardsTotal ?? 0;
  const done = item.cardsDone ?? 0;
  const progress = total > 0 ? Math.min(1, done / total) : 0;
  const agentsActive = item.agentsActive ?? 0;
  const subLine = item.kind === "chat" ? chatSubLine(item) : null;

  const chips: string[] = [];
  if (item.kind === "chat") {
    if (item.mode) chips.push(item.mode);
    if (item.model) chips.push(item.model);
    if (item.diff) chips.push(`+${item.diff.adds} −${item.diff.dels}`);
    if (item.totalTokens != null && item.totalTokens > 0) {
      chips.push(`${formatCompactNumber(item.totalTokens)} tok`);
    }
    if (item.costUsd != null && item.costUsd > 0) {
      chips.push(`$${item.costUsd.toFixed(2)}`);
    }
  } else {
    chips.push(item.status);
    if (total > 0) chips.push(`${done}/${total}`);
    if (item.branch) chips.push(item.branch);
    if (item.linkedChats) chips.push(`${item.linkedChats} linked`);
  }

  return (
    <div
      role="button"
      tabIndex={0}
      className={`${styles.card} rf-enter-rise rf-pressable`}
      data-kind={item.kind}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen();
        }
      }}
    >
      <div className={styles.cardTop}>
        <span className={styles.kindTile}>
          <Icon
            icon={item.kind === "chat" ? MessageSquare : ListChecks}
            size="md"
            tone={item.kind === "chat" ? "accent" : "success"}
          />
        </span>
        <span className={styles.title}>{item.title}</span>
        {live ? (
          <span className={styles.liveChip}>
            <span className={styles.liveDot} />
            {liveLabel(item)}
          </span>
        ) : null}
      </div>

      {subLine ? <span className={styles.subLine}>{subLine}</span> : null}

      {item.kind === "task" && total > 0 ? (
        <div className={styles.progressTrack}>
          <div
            className={styles.progressFill}
            data-shimmer={agentsActive > 0 || undefined}
            style={
              {
                "--rf-continue-progress": `${Math.round(progress * 100)}%`,
              } as React.CSSProperties
            }
          />
        </div>
      ) : null}

      <div className={styles.chipRow}>
        {chips.map((chip) => (
          <span className={styles.chip} key={chip}>
            {chip}
          </span>
        ))}
        <span className={styles.time}>
          {formatRelativeTime(item.updatedAtMs)}
        </span>
      </div>
    </div>
  );
};

export const ContinueSection: React.FC<ContinueSectionProps> = ({
  onOpenChat,
  onOpenTask,
}) => {
  const items = useAppSelector(selectContinueItems);

  if (items.length === 0) return null;

  return (
    <section className={styles.section} aria-label="Continue">
      <div className={`${styles.grid} rf-stagger`}>
        {items.map((item) => (
          <ContinueCard
            key={`${item.kind}:${item.id}`}
            item={item}
            onOpen={() =>
              item.kind === "chat" ? onOpenChat(item.id) : onOpenTask(item.id)
            }
          />
        ))}
      </div>
    </section>
  );
};
