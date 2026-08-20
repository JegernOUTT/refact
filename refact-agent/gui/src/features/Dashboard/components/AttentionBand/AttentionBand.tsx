import React from "react";
import { AlertTriangle } from "lucide-react";
import { Icon } from "../../../../components/ui";
import { useAppSelector } from "../../../../hooks";
import { selectAttentionItems, type StreamItem } from "../../streamSelectors";
import styles from "./AttentionBand.module.css";

const MAX_ROWS = 3;

type AttentionBandProps = {
  onOpenChat: (id: string) => void;
  onOpenTask: (id: string) => void;
};

const STATUS_REASON: Partial<Record<string, string>> = {
  failed: "failed",
  paused: "paused",
  planning: "waiting on plan",
  idle: "idle",
  done: "done",
  streaming: "streaming",
  working: "working",
};

function reasonFor(item: StreamItem): string {
  const failed = item.cardsFailed ?? 0;
  if (item.kind === "task" && failed > 0) {
    return `${failed} card${failed === 1 ? "" : "s"} failed`;
  }
  return STATUS_REASON[item.status] ?? item.status;
}

export const AttentionBand: React.FC<AttentionBandProps> = ({
  onOpenChat,
  onOpenTask,
}) => {
  const items = useAppSelector(selectAttentionItems);

  if (items.length === 0) return null;

  const visible = items.slice(0, MAX_ROWS);
  const overflow = items.length - visible.length;

  return (
    <section className={`${styles.band} rf-enter`} aria-label="Needs attention">
      {visible.map((item) => (
        <button
          type="button"
          key={`${item.kind}:${item.id}`}
          className={`${styles.row} rf-pressable`}
          onClick={() =>
            item.kind === "chat" ? onOpenChat(item.id) : onOpenTask(item.id)
          }
        >
          <Icon icon={AlertTriangle} size="sm" tone="warning" />
          <span className={styles.name}>{item.title}</span>
          <span className={styles.reason}>{reasonFor(item)}</span>
        </button>
      ))}
      {overflow > 0 ? (
        <span className={styles.overflow}>and {overflow} more</span>
      ) : null}
    </section>
  );
};
