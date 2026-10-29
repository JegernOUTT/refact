import type { RootState } from "../../../app/store";
import { useThreadId } from "../../../features/Chat/Thread";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
  selectVisibleMessages,
} from "../../../features/Chat/Thread/selectors";
import { useAppSelector } from "../../../hooks/useAppSelector";

export function selectIsToolCallInFlight(
  state: RootState,
  threadId: string,
  toolCallId: string | undefined,
): boolean {
  if (
    !toolCallId ||
    !(
      selectIsStreamingById(state, threadId) ||
      selectIsWaitingById(state, threadId)
    )
  ) {
    return false;
  }
  const messages = selectVisibleMessages(state, threadId);
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index];
    if (message.role === "user") return false;
    if (message.role === "assistant") {
      const owns = (call: { id?: string }) => call.id === toolCallId;
      return (
        message.tool_calls?.some(owns) === true ||
        message.server_executed_tools?.some(owns) === true
      );
    }
  }
  return false;
}

export function useToolCallActivity(toolCallId?: string, threadId?: string) {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, resolvedThreadId, toolCallId),
  );
  const isRunning = useAppSelector(
    (state) =>
      !selectToolResultByThreadAndId(state, resolvedThreadId, toolCallId) &&
      selectIsToolCallInFlight(state, resolvedThreadId, toolCallId),
  );
  return { result, isRunning, threadId: resolvedThreadId };
}
