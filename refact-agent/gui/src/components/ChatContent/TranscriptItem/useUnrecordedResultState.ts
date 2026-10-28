import { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useThreadId } from "../../../features/Chat/Thread";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectThreadPauseReasonsById,
} from "../../../features/Chat/Thread/selectors";
import {
  unrecordedResultState,
  type UnrecordedResultState,
} from "./unrecordedResult";

export function useUnrecordedResultState(
  toolCallId: string | undefined,
): UnrecordedResultState {
  const threadId = useThreadId();
  const busy = useAppSelector(
    (state) =>
      selectIsStreamingById(state, threadId) ||
      selectIsWaitingById(state, threadId),
  );
  const pause = useAppSelector((state) => {
    if (!toolCallId) return null;
    const reason = selectThreadPauseReasonsById(state, threadId).find(
      (item) => item.tool_call_id === toolCallId,
    );
    return reason?.type ?? null;
  });
  return useMemo(() => unrecordedResultState({ busy, pause }), [busy, pause]);
}
