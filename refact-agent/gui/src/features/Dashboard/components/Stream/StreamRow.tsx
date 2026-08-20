import React, { useCallback } from "react";
import classNames from "classnames";
import {
  ChevronDown,
  GitBranch,
  LayoutGrid,
  MessageSquare,
} from "lucide-react";
import { Icon } from "../../../../components/ui";
import { humanizeIdentifier } from "../../../../utils/displayNames";
import type { StreamItem } from "../../streamSelectors";
import {
  formatRelativeTime,
  staggerClass,
  statusTone,
  TONE_CLASS,
} from "./streamRowUtils";
import styles from "./Stream.module.css";

export type StreamRowProps = {
  item: StreamItem;
  index: number;
  isPeekOpen: boolean;
  isFamilyOpen: boolean;
  onOpen: (item: StreamItem) => void;
  onTogglePeek: (id: string) => void;
  onToggleFamily: (id: string) => void;
};

export const StreamRow: React.FC<StreamRowProps> = ({
  item,
  index,
  isPeekOpen,
  isFamilyOpen,
  onOpen,
  onTogglePeek,
  onToggleFamily,
}) => {
  const isChat = item.kind === "chat";

  const handleClick = useCallback(() => {
    onOpen(item);
  }, [item, onOpen]);

  const handleExpandClick = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      onTogglePeek(item.id);
    },
    [item.id, onTogglePeek],
  );

  const handleFamilyClick = useCallback(
    (event: React.MouseEvent<HTMLSpanElement>) => {
      event.stopPropagation();
      onToggleFamily(item.id);
    },
    [item.id, onToggleFamily],
  );

  const handleFamilyKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLSpanElement>) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      event.stopPropagation();
      onToggleFamily(item.id);
    },
    [item.id, onToggleFamily],
  );

  const tone = statusTone(item.status);

  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={isPeekOpen}
      data-open={isPeekOpen ? "true" : undefined}
      data-testid={`stream-row-${item.id}`}
      className={classNames(styles.row, staggerClass(index))}
      onClick={handleClick}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          handleClick();
        }
      }}
    >
      <span
        className={classNames(
          styles.tile,
          isChat ? styles.tileChat : styles.tileTask,
        )}
      >
        <Icon
          icon={isChat ? MessageSquare : LayoutGrid}
          size="sm"
          tone={isChat ? "accent" : "success"}
        />
      </span>

      <span className={styles.titleCell}>
        <span className={styles.title}>{item.title}</span>
        {item.familyChildCount > 0 && (
          <span
            role="button"
            tabIndex={0}
            aria-expanded={isFamilyOpen}
            aria-label={`Toggle thread family for ${item.title}`}
            className={styles.familyPill}
            onClick={handleFamilyClick}
            onKeyDown={handleFamilyKeyDown}
          >
            <Icon icon={GitBranch} size="sm" tone="accent" />
            {item.familyChildCount}
          </span>
        )}
        {isChat && item.messageCount != null && item.messageCount > 0 && (
          <span className={styles.msgCount}>{item.messageCount}</span>
        )}
      </span>

      <span className={classNames(styles.slot, styles.slot1)}>
        {isChat ? (
          <>
            {item.mode && <span className={styles.chip}>{item.mode}</span>}
            {item.diff && (
              <span className={classNames(styles.chip, styles.chipMono)}>
                <span className={styles.chipAdds}>+{item.diff.adds}</span>
                <span className={styles.chipDels}>−{item.diff.dels}</span>
              </span>
            )}
          </>
        ) : (
          <>
            <span className={styles.chip}>
              {item.cardsDone}/{item.cardsTotal}
            </span>
            {item.cardsFailed != null && item.cardsFailed > 0 && (
              <span className={classNames(styles.chip, styles.chipDels)}>
                {item.cardsFailed} failed
              </span>
            )}
          </>
        )}
      </span>

      <span className={styles.slot}>
        <span className={classNames(styles.stateChip, TONE_CLASS[tone])}>
          {humanizeIdentifier(item.status)}
        </span>
      </span>

      <button
        type="button"
        className={styles.expandBtn}
        aria-label={`Toggle details for ${item.title}`}
        aria-expanded={isPeekOpen}
        data-testid={`stream-expand-${item.id}`}
        onClick={handleExpandClick}
      >
        <Icon icon={ChevronDown} size="sm" />
      </button>

      <span className={styles.time}>
        {formatRelativeTime(item.updatedAtMs)}
      </span>
    </div>
  );
};
