import { Pause, TriangleAlert } from "lucide-react";
import { Badge, Icon, Tooltip } from "../../components/ui";
import {
  normalizeCardAttention,
  type BoardCard,
} from "../../services/refact/tasks";
import styles from "./CardAttentionBadge.module.css";

interface CardAttentionBadgeProps {
  card: Pick<BoardCard, "attention" | "paused_at" | "paused_by">;
}

export function CardAttentionBadge({ card }: CardAttentionBadgeProps) {
  const attention = normalizeCardAttention(card.attention);
  const pausedAt =
    typeof card.paused_at === "string" ? card.paused_at : undefined;
  const pausedBy =
    typeof card.paused_by === "string" ? card.paused_by : undefined;
  if (!attention && !pausedAt) return null;

  const attentionLabel = attention
    ? `Needs attention: ${attention.reason} · Kind: ${attention.kind} · Since: ${attention.since} · ×${attention.count}`
    : "";
  const pausedLabel = `Paused since ${pausedAt}${
    pausedBy ? ` by ${pausedBy}` : ""
  }`;

  return (
    <>
      {attention && (
        <Tooltip content={attentionLabel}>
          <Badge
            className={styles.badge}
            tone="warning"
            size="xs"
            role="status"
            aria-label={attentionLabel}
            tabIndex={0}
          >
            <Icon icon={TriangleAlert} size="sm" tone="warning" />
            Needs attention
          </Badge>
        </Tooltip>
      )}
      {pausedAt && (
        <Tooltip content={pausedLabel}>
          <Badge
            className={styles.badge}
            tone="muted"
            size="xs"
            role="status"
            aria-label={pausedLabel}
            tabIndex={0}
          >
            <Icon icon={Pause} size="sm" tone="muted" />
            Paused
          </Badge>
        </Tooltip>
      )}
    </>
  );
}
