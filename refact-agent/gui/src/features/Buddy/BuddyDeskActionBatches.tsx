import { useEffect } from "react";
import { useGetBuddyDeskActionsBatchQuery } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import type { ChunkActions } from "./BuddyDeskModel";

function errorStatus(error: unknown): unknown {
  return error && typeof error === "object" && "status" in error
    ? error.status
    : undefined;
}

function ActionBatchSubscriber({
  chunkKey,
  chunk,
  endpointIdentity,
  onResult,
}: {
  chunkKey: string;
  chunk: WorkItem[];
  endpointIdentity: string;
  onResult: (key: string, result: ChunkActions | null) => void;
}) {
  const projectId = chunk[0]?.project.project_id ?? "";
  const ids = chunk
    .filter(
      (value) =>
        Boolean(value.current_artifact_id) &&
        value.project.project_id === projectId,
    )
    .map((value) => value.id);
  const batch = useGetBuddyDeskActionsBatchQuery(
    { ids, project_id: projectId, endpoint_identity: endpointIdentity },
    { skip: ids.length === 0 },
  );
  const { currentData, isFetching, error } = batch;
  useEffect(() => {
    const status = errorStatus(error);
    const perRow = status === 404 || status === 405;
    const result: ChunkActions = {};
    for (const item of chunk) {
      result[item.id] = perRow
        ? { mode: "per_row" }
        : {
            mode: "batch",
            entry: currentData?.data.find((entry) => entry.work_id === item.id),
            loaded: currentData !== undefined,
            pending: isFetching && !currentData,
            failed:
              error !== undefined || item.project.project_id !== projectId,
          };
    }
    onResult(chunkKey, result);
  }, [chunk, chunkKey, currentData, error, isFetching, onResult, projectId]);
  useEffect(() => () => onResult(chunkKey, null), [chunkKey, onResult]);
  return null;
}

export function BuddyDeskActionBatches({
  chunks,
  endpointIdentity,
  onResult,
}: {
  chunks: { key: string; items: WorkItem[] }[];
  endpointIdentity: string;
  onResult: (key: string, result: ChunkActions | null) => void;
}) {
  return (
    <>
      {chunks.map((chunk) => (
        <ActionBatchSubscriber
          key={chunk.key}
          chunkKey={chunk.key}
          chunk={chunk.items}
          endpointIdentity={endpointIdentity}
          onResult={onResult}
        />
      ))}
    </>
  );
}
