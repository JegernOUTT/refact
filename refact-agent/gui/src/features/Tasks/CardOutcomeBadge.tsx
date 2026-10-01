import { Badge, Tooltip } from "../../components/ui";
import type { CardOutcome } from "../../services/refact/tasks";
import styles from "./CardOutcomeBadge.module.css";

interface CardOutcomeBadgeProps {
  cardId: string;
  cardOutcomes?: Record<string, CardOutcome>;
}

export function CardOutcomeBadge({
  cardId,
  cardOutcomes,
}: CardOutcomeBadgeProps) {
  const outcome = cardOutcomes?.[cardId];
  if (!outcome) return null;

  const finalized = outcome.finalized;
  const requiredCount = outcome.required.length;
  const recordedCount = outcome.required.filter((requirement) =>
    outcome.fragments.some(
      (fragment) => fragment.name === requirement.fragment,
    ),
  ).length;
  const label = finalized
    ? `Finalized by ${finalized.finalized_by} at ${finalized.finalized_at}`
    : `Outcome: ${recordedCount}/${requiredCount} fragments`;

  const badge = (
    <Badge
      className={styles.badge}
      tone={finalized ? "success" : "muted"}
      size="xs"
      role="status"
      aria-label={label}
      tabIndex={0}
    >
      {finalized ? "Finalized" : label}
    </Badge>
  );

  return finalized ? <Tooltip content={label}>{badge}</Tooltip> : badge;
}
