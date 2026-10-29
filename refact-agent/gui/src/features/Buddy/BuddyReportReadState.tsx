import { useRef, useState } from "react";
import { Button } from "../../components/ui";
import { BuddyStatusBadge } from "./ui";
import styles from "./BuddyDeskDetail.module.css";
import {
  useGetBuddyWorkReadStateQuery,
  useMarkBuddyWorkReadMutation,
} from "../../services/refact/buddyOperator";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";

export function BuddyReportReadState({
  item,
  artifact,
  endpointIdentity,
}: {
  item: WorkItem;
  artifact: ArtifactRevision;
  endpointIdentity: string;
}) {
  const args = {
    id: item.id,
    project_id: item.project.project_id,
    canonical_root: item.project.canonical_root,
    endpoint_identity: endpointIdentity,
  };
  const { currentData, error, isFetching, refetch } =
    useGetBuddyWorkReadStateQuery(args, { refetchOnMountOrArgChange: true });
  const [markRead, result] = useMarkBuddyWorkReadMutation();
  const pending = useRef(false);
  const [failed, setFailed] = useState(false);
  const state =
    result.data?.data &&
    currentData?.data &&
    result.data.data.artifact_id === currentData.data.artifact_id &&
    currentData.data.last_read_revision > result.data.data.last_read_revision
      ? currentData.data
      : result.data?.data ?? currentData?.data;
  const read =
    state?.artifact_id === artifact.artifact_id &&
    state.last_read_revision >= artifact.revision;
  const mark = async () => {
    if (pending.current || Boolean(error) || isFetching || read) return;
    pending.current = true;
    setFailed(false);
    try {
      await markRead({
        ...args,
        artifact_id: artifact.artifact_id,
        artifact_revision: artifact.revision,
      }).unwrap();
    } catch {
      setFailed(true);
    } finally {
      pending.current = false;
    }
  };
  return (
    <section
      aria-label="Report reading status"
      className={styles.inlineControl}
    >
      {!error && !isFetching && (
        <BuddyStatusBadge dimension="read" value={read ? "read" : "unread"} />
      )}
      <p className={styles.hint}>
        {read
          ? "This revision is marked read."
          : error
            ? "Couldn't load reading status."
            : isFetching
              ? "Loading reading status…"
              : "New revision since you last read it."}
      </p>
      {error && (
        <Button size="sm" variant="ghost" onClick={() => void refetch()}>
          Retry reading status
        </Button>
      )}
      {!read && (
        <Button
          size="sm"
          variant="ghost"
          disabled={result.isLoading || isFetching || Boolean(error)}
          onClick={() => void mark()}
        >
          {result.isLoading ? "Saving…" : "Mark as read"}
        </Button>
      )}
      {failed && (
        <p role="alert" className={styles.hint}>
          Couldn&apos;t save reading status. Refresh the report and try again.
        </p>
      )}
    </section>
  );
}
