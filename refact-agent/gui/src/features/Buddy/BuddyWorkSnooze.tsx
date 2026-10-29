import { useRef, useState } from "react";
import { Button } from "../../components/ui";
import {
  useGetBuddyWorkSnoozeQuery,
  useSnoozeBuddyWorkMutation,
  type WorkSnooze,
} from "../../services/refact/buddyOperator";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";
import { BuddyTimestamp } from "./ui";
import styles from "./BuddyDeskDetail.module.css";

export function BuddyWorkSnooze({
  item,
  artifact,
  identity,
  onChange,
}: {
  item: WorkItem;
  artifact: ArtifactRevision;
  identity: string;
  onChange: () => void;
}) {
  const args = {
    id: item.id,
    project_id: item.project.project_id,
    canonical_root: item.project.canonical_root,
    endpoint_identity: identity,
  };
  const query = useGetBuddyWorkSnoozeQuery(args, {
    refetchOnMountOrArgChange: true,
  });
  const [save, result] = useSnoozeBuddyWorkMutation();
  const [failed, setFailed] = useState(false);
  const [savedSnooze, setSavedSnooze] = useState<WorkSnooze | null>();
  const pending = useRef(false);
  const snooze =
    savedSnooze !== undefined
      ? savedSnooze
      : result.data === undefined
        ? query.currentData?.data
        : result.data.data;
  const active =
    snooze?.artifact_id === artifact.artifact_id &&
    snooze.artifact_revision === artifact.revision &&
    Date.parse(snooze.until) > Date.now();
  const statusKnown =
    query.currentData !== undefined ||
    result.data !== undefined ||
    savedSnooze !== undefined;
  const loading = !statusKnown && !query.error;
  const change = async (minutes: number | null) => {
    if (pending.current || loading || query.isFetching || Boolean(query.error))
      return;
    pending.current = true;
    setFailed(false);
    try {
      const response = await save({
        ...args,
        artifact_id: artifact.artifact_id,
        artifact_revision: artifact.revision,
        minutes,
      }).unwrap();
      setSavedSnooze(response.data);
      onChange();
    } catch {
      setFailed(true);
    } finally {
      pending.current = false;
    }
  };
  return (
    <section aria-label="Work snooze" className={styles.inlineControl}>
      {loading ? (
        <p role="status" className={styles.hint}>
          Loading snooze status…
        </p>
      ) : statusKnown && active ? (
        <p className={styles.hint}>
          Snoozed until <BuddyTimestamp value={snooze.until} mode="absolute" />.
          A new revision ends the snooze.
        </p>
      ) : null}
      {query.error ? (
        <>
          <p role="alert" className={styles.hint}>
            Couldn&apos;t load snooze status.
          </p>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void query.refetch()}
          >
            Retry snooze status
          </Button>
        </>
      ) : (
        <>
          <Button
            size="sm"
            variant="ghost"
            disabled={loading || query.isFetching || result.isLoading}
            onClick={() => void change(active ? null : 60)}
          >
            {active ? "End snooze" : "Snooze for one hour"}
          </Button>
          {!active && (
            <Button
              size="sm"
              variant="ghost"
              disabled={loading || query.isFetching || result.isLoading}
              onClick={() => void change(1440)}
            >
              Snooze for one day
            </Button>
          )}
        </>
      )}
      {result.isLoading && (
        <p role="status" className={styles.hint}>
          Saving snooze…
        </p>
      )}
      {failed && (
        <p role="alert" className={styles.hint}>
          Couldn&apos;t save the snooze. Refresh the report and try again.
        </p>
      )}
    </section>
  );
}
