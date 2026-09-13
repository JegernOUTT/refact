import { useRef, useState } from "react";
import { Button } from "../../components/ui";
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
    <section aria-label="Report reading status">
      <p>
        {read
          ? "This report revision is marked read."
          : error
            ? "Saved reading status is unavailable."
            : isFetching
              ? "Loading reading status…"
              : "New report revision since last read."}
      </p>
      <p>Reading does not accept a proposal or resolve this work.</p>
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
          {result.isLoading
            ? "Saving reading status…"
            : "Mark this revision read"}
        </Button>
      )}
      {failed && (
        <p role="alert">
          Reading status was not acknowledged. Reload the report if it changed,
          then try again.
        </p>
      )}
    </section>
  );
}
