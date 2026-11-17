import { useCallback, useState } from "react";
import type { OperatorRecord } from "../../../services/refact/buddyOperator";
import type { WorkItem } from "../operatorTypes";
import {
  PENDING_ROW_ACTIONS,
  chunkWork,
  sameChunkActions,
  type ChunkActions,
  type RowActions,
} from "../BuddyDeskModel";

function withChunk(
  current: Partial<Record<string, ChunkActions>>,
  key: string,
  result: ChunkActions | null,
): Partial<Record<string, ChunkActions>> {
  if (result === null) {
    if (!(key in current)) return current;
    return Object.fromEntries(
      Object.entries(current).filter(([entry]) => entry !== key),
    );
  }
  if (sameChunkActions(current[key], result)) return current;
  return { ...current, [key]: result };
}

export function useDeskRowActions({
  filtered,
  endpointIdentity,
}: {
  filtered: OperatorRecord<WorkItem>[];
  endpointIdentity: string;
}) {
  const [actionVerbs, setActionVerbs] = useState<
    Partial<Record<string, string>>
  >({});
  const [chunkActions, setChunkActions] = useState<
    Partial<Record<string, ChunkActions>>
  >({});
  const recordChunkActions = useCallback(
    (key: string, result: ChunkActions | null) => {
      setChunkActions((current) => withChunk(current, key, result));
    },
    [],
  );
  const recordActionVerb = useCallback((id: string, verb: string) => {
    setActionVerbs((current) =>
      current[id] === verb ? current : { ...current, [id]: verb },
    );
  }, []);
  const actionChunks = chunkWork(filtered, endpointIdentity);
  const chunkByWork = new Map(
    actionChunks.flatMap((chunk) =>
      chunk.items.map((value) => [value.id, chunk.key] as const),
    ),
  );
  const rowActionsFor = (id: string): RowActions => {
    const chunkKey = chunkByWork.get(id);
    const chunk = chunkKey === undefined ? undefined : chunkActions[chunkKey];
    return chunk && id in chunk ? chunk[id] : PENDING_ROW_ACTIONS;
  };
  const actionVerbsReady =
    filtered.length > 0 &&
    filtered.every(({ value }) => actionVerbs[value.id] !== undefined);
  const showActionVerb =
    actionVerbsReady &&
    new Set(filtered.map(({ value }) => actionVerbs[value.id])).size > 1;
  return {
    actionChunks,
    recordChunkActions,
    recordActionVerb,
    rowActionsFor,
    showActionVerb,
  };
}

export type DeskRowActions = ReturnType<typeof useDeskRowActions>;
