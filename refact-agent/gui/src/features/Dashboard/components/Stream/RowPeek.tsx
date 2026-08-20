import React from "react";
import classNames from "classnames";
import { Button } from "../../../../components/ui";
import { DeletePopover } from "../../../../components/DeletePopover/DeletePopover";
import type { StreamItem } from "../../streamSelectors";
import { formatCompactNumber, staggerClass } from "./streamRowUtils";
import styles from "./Stream.module.css";

function formatDateTime(timestampMs: number): string {
  return new Date(timestampMs).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function tokensLabel(item: StreamItem): string | null {
  if (item.totalTokens == null) return null;
  const base = formatCompactNumber(item.totalTokens);
  if (item.cacheReadTokens == null || item.totalTokens === 0) {
    return base;
  }
  const pct = Math.round((item.cacheReadTokens / item.totalTokens) * 100);
  return `${base} · ${pct}% cached`;
}

function modelLabel(item: StreamItem): string | null {
  if (!item.model && !item.mode) return null;
  if (item.model && item.mode) return `${item.model} · ${item.mode}`;
  return item.model ?? item.mode ?? null;
}

export type RowPeekProps = {
  item: StreamItem;
  onOpen: () => void;
  onDelete: () => void;
};

export const RowPeek: React.FC<RowPeekProps> = ({ item, onOpen, onDelete }) => {
  const blocks: { label: string; value: React.ReactNode }[] = [];

  const model = modelLabel(item);
  if (model !== null) blocks.push({ label: "Model", value: model });

  const tokens = tokensLabel(item);
  if (tokens !== null) blocks.push({ label: "Tokens", value: tokens });

  if (item.costUsd != null) {
    blocks.push({ label: "Cost", value: `$${item.costUsd.toFixed(2)}` });
  }

  if (item.messageCount != null) {
    blocks.push({ label: "Messages", value: String(item.messageCount) });
  }

  if (item.diff) {
    blocks.push({
      label: "Diff",
      value: (
        <span className={styles.pkDiff}>
          <span className={styles.pkAdds}>+{item.diff.adds}</span>
          <span className={styles.pkDels}>−{item.diff.dels}</span>
        </span>
      ),
    });
  }

  if (item.familyChildCount > 0) {
    blocks.push({
      label: item.familyChildCount === 1 ? "Subchat" : "Subchats",
      value: String(item.familyChildCount),
    });
  }

  if (item.kind === "task" && item.cardsTotal != null && item.cardsTotal > 0) {
    const failed =
      item.cardsFailed != null && item.cardsFailed > 0
        ? ` · ${item.cardsFailed} failed`
        : "";
    blocks.push({
      label: "Cards",
      value: `${item.cardsDone ?? 0}/${item.cardsTotal}${failed}`,
    });
  }

  if (item.agentsActive != null && item.agentsActive > 0) {
    blocks.push({ label: "Agents", value: String(item.agentsActive) });
  }

  if (item.linkedChats != null && item.linkedChats > 0) {
    blocks.push({ label: "Linked chats", value: String(item.linkedChats) });
  }

  if (item.branch) blocks.push({ label: "Branch", value: item.branch });

  if (item.createdAtMs && item.updatedAtMs) {
    blocks.push({
      label: "Created → Updated",
      value: `${formatDateTime(item.createdAtMs)} → ${formatDateTime(
        item.updatedAtMs,
      )}`,
    });
  }

  return (
    <div className={styles.peek} data-testid={`stream-peek-${item.id}`}>
      <div className={styles.peekBlocks}>
        {blocks.map((block, index) => (
          <div
            key={block.label}
            className={classNames(styles.pk, staggerClass(index))}
          >
            <span className={styles.pkLabel}>{block.label}</span>
            <span className={styles.pkValue}>{block.value}</span>
          </div>
        ))}
      </div>

      <div className={styles.peekActions}>
        <Button variant="primary" size="sm" onClick={onOpen}>
          Open
        </Button>
        {item.kind === "chat" && (
          <DeletePopover
            size="sm"
            triggerClassName={styles.peekDelete}
            itemName={item.title || "this item"}
            deleteBy={item.id}
            isDisabled={false}
            isDeleting={false}
            handleDelete={() => onDelete()}
          />
        )}
      </div>
    </div>
  );
};
