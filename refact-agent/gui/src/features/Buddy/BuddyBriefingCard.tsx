import React, { useState } from "react";
import classNames from "classnames";
import { Sunrise, Undo2 } from "lucide-react";
import { Badge, Button, Surface, Text } from "../../components/ui";
import {
  useGetBuddyBriefingQuery,
  useUndoBuddyActionMutation,
} from "../../services/refact/buddy";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { formatOpportunityActionError } from "./hooks/useExecuteBuddyAction";
import styles from "./BuddyBriefingCard.module.css";

export const BuddyBriefingCard: React.FC = () => {
  const { data: briefing } = useGetBuddyBriefingQuery(undefined);
  const [undoAction] = useUndoBuddyActionMutation();
  const [undoError, setUndoError] = useState<string | null>(null);

  if (!briefing) return null;

  const activeReceipts = briefing.receipts.filter((r) => !r.undone);
  const spendTokens = briefing.spend.tokens_in + briefing.spend.tokens_out;

  const handleUndo = async (receiptId: string) => {
    setUndoError(null);
    try {
      await undoAction({ receipt_id: receiptId }).unwrap();
    } catch (error) {
      setUndoError(`Undo failed: ${formatOpportunityActionError(error)}`);
    }
  };

  return (
    <Surface
      className={styles.card}
      data-testid="buddy-briefing-card"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader
        icon={Sunrise}
        label={`Briefing — ${briefing.date}`}
        badge={
          briefing.top_cards.length > 0 ? (
            <Badge size="xs" tone="accent">
              {briefing.top_cards.length} to review
            </Badge>
          ) : undefined
        }
      />
      <div className={classNames(styles.body, "rf-stagger")}>
        {briefing.top_cards.length > 0 && (
          <div className={styles.section}>
            <Text size="1" weight="bold" color="gray">
              TOP CARDS
            </Text>
            {briefing.top_cards.map((card) => (
              <div key={card.id} className={styles.row}>
                <Badge size="xs" tone="muted">
                  {card.priority}
                </Badge>
                <Text size="1" className={styles.rowText}>
                  {card.summary}
                </Text>
              </div>
            ))}
          </div>
        )}
        {activeReceipts.length > 0 && (
          <div className={styles.section}>
            <Text size="1" weight="bold" color="gray">
              APPLIED CHANGES
            </Text>
            {activeReceipts.map((receipt) => (
              <div key={receipt.id} className={styles.row}>
                <Text size="1" className={styles.rowText}>
                  {receipt.target_path}
                </Text>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Undo ${receipt.target_path}`}
                  onClick={() => void handleUndo(receipt.id)}
                >
                  <Undo2 size={14} />
                  Undo
                </Button>
              </div>
            ))}
          </div>
        )}
        <div className={styles.statsRow}>
          <Text size="1" color="gray">
            {briefing.job_runs.length} job(s) ran
          </Text>
          <Text size="1" color="gray">
            {briefing.pulse.diagnostics_last_hour} diagnostics/h
          </Text>
          <Text size="1" color="gray">
            {spendTokens.toLocaleString()} tokens today
          </Text>
        </div>
        {undoError && (
          <Text size="1" color="red" role="alert">
            {undoError}
          </Text>
        )}
      </div>
    </Surface>
  );
};
