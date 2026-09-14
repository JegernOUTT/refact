import React, { useState } from "react";
import classNames from "classnames";
import { FileClock, Undo2 } from "lucide-react";
import { Badge, Button, Surface, Text } from "../../components/ui";
import {
  useGetBuddyReceiptsQuery,
  useUndoBuddyActionMutation,
} from "../../services/refact/buddy";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { formatTimestamp } from "./buddyUtils";
import { formatOpportunityActionError } from "./hooks/useExecuteBuddyAction";
import styles from "./BuddyAppliedChangesCard.module.css";

export const BuddyAppliedChangesCard: React.FC = () => {
  const { data: receipts, isError } = useGetBuddyReceiptsQuery(undefined);
  const [undoAction] = useUndoBuddyActionMutation();
  const [undoError, setUndoError] = useState<string | null>(null);

  if (!isError && !receipts) return null;

  const undoable = (receipts ?? []).filter((receipt) => !receipt.undone);

  const handleUndo = async (receiptId: string) => {
    setUndoError(null);
    try {
      await undoAction({ receipt_id: receiptId }).unwrap();
    } catch (error) {
      setUndoError(`Could not undo: ${formatOpportunityActionError(error)}`);
    }
  };

  return (
    <Surface
      className={styles.card}
      data-testid="buddy-applied-changes-card"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader
        icon={FileClock}
        label="Files Buddy changed"
        badge={
          undoable.length > 0 ? (
            <Badge size="xs" tone="accent">
              {undoable.length}
            </Badge>
          ) : undefined
        }
      />
      <div className={classNames(styles.body, "rf-stagger")}>
        {isError && (
          <Text size="1" color="gray">
            That list could not be loaded just now.
          </Text>
        )}
        {!isError && undoable.length === 0 && (
          <Text size="1" color="gray">
            Buddy has not changed any of your files yet.
          </Text>
        )}
        {undoable.map((receipt) => (
          <div key={receipt.id} className={styles.row}>
            <span className={styles.rowText}>
              <Text
                size="1"
                className={styles.path}
                title={receipt.target_path}
              >
                {receipt.target_path}
              </Text>
              <Text size="1" color="gray">
                {formatTimestamp(receipt.created_at)}
              </Text>
            </span>
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
        {undoError && (
          <Text size="1" color="red" role="alert">
            {undoError}
          </Text>
        )}
      </div>
    </Surface>
  );
};
