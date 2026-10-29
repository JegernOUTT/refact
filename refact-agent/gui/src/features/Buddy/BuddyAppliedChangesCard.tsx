import React, { useState } from "react";
import { FileClock, FileText, Undo2 } from "lucide-react";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  useGetBuddyReceiptsQuery,
  useUndoBuddyActionMutation,
} from "../../services/refact/buddy";
import { selectBuddyState } from "./buddySlice";
import { formatOpportunityActionError } from "./hooks/useExecuteBuddyAction";
import {
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyRow,
  BuddySection,
  BuddyTimestamp,
  buddyName,
} from "./ui";
import styles from "./BuddyAppliedChangesCard.module.css";

export const BuddyAppliedChangesCard: React.FC = () => {
  const name = buddyName(useAppSelector(selectBuddyState)?.identity.name);
  const {
    data: receipts,
    isError,
    error,
    isFetching,
    refetch,
  } = useGetBuddyReceiptsQuery(undefined);
  const [undoAction] = useUndoBuddyActionMutation();
  const [undoError, setUndoError] = useState<string | null>(null);

  if (!isError && !receipts) return null;

  const undoable = (receipts ?? []).filter((receipt) => !receipt.undone);

  const handleUndo = async (receiptId: string) => {
    setUndoError(null);
    try {
      await undoAction({ receipt_id: receiptId }).unwrap();
    } catch (undoFailure) {
      setUndoError(
        `Could not undo: ${formatOpportunityActionError(undoFailure)}`,
      );
    }
  };

  return (
    <BuddySection
      title={`Files ${name} changed`}
      icon={FileClock}
      count={undoable.length > 0 ? undoable.length : undefined}
      testId="buddy-applied-changes-card"
    >
      {isError ? (
        <BuddyErrorCallout
          compact
          thing="changed files"
          error={error}
          retrying={isFetching}
          onRetry={() => void refetch()}
        />
      ) : null}
      {!isError && undoable.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={FileClock}
          title={`${name} has not changed any of your files yet.`}
        />
      ) : null}
      {undoable.length > 0 ? (
        <div className={styles.body}>
          {undoable.map((receipt) => (
            <BuddyRow
              key={receipt.id}
              density="compact"
              leading={{ icon: FileText }}
              title={receipt.target_path}
              meta={[
                <BuddyTimestamp
                  key="at"
                  value={receipt.created_at}
                  prefix="Changed"
                />,
              ]}
              actions={
                <Button
                  size="sm"
                  variant="ghost"
                  leftIcon={Undo2}
                  aria-label={`Undo ${receipt.target_path}`}
                  onClick={() => void handleUndo(receipt.id)}
                >
                  Undo
                </Button>
              }
            />
          ))}
        </div>
      ) : null}
      {undoError ? (
        <p className={styles.error} role="alert">
          {undoError}
        </p>
      ) : null}
    </BuddySection>
  );
};
