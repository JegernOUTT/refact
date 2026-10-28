import { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useThreadId } from "../../../features/Chat/Thread";
import { selectThreadConfirmationById } from "../../../features/Chat/Thread/selectors";
import { selectIsToolCallInFlight } from "../ToolCard/useToolCallActivity";
import {
  unrecordedResultState,
  type UnrecordedResultState,
} from "./unrecordedResult";

export function useUnrecordedResultState(
  toolCallId: string | undefined,
  threadId?: string,
): UnrecordedResultState {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const busy = useAppSelector((state) =>
    selectIsToolCallInFlight(state, resolvedThreadId, toolCallId),
  );
  const pause = useAppSelector((state) => {
    if (!toolCallId) return null;
    const reasons = selectThreadConfirmationById(
      state,
      resolvedThreadId,
    ).pause_reasons;
    const reason = reasons.find((item) => item.tool_call_id === toolCallId);
    return reason?.type ?? null;
  });
  return useMemo(() => unrecordedResultState({ busy, pause }), [busy, pause]);
}
