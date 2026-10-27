import { useThreadId } from "../../../features/Chat/Thread";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
  selectVisibleMessages,
} from "../../../features/Chat/Thread/selectors";
import { useAppSelector } from "../../../hooks/useAppSelector";

export function useToolCallActivity(toolCallId?: string, threadId?: string) {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, resolvedThreadId, toolCallId),
  );
  const isRunning = useAppSelector((state) => {
    if (
      !toolCallId ||
      Boolean(
        selectToolResultByThreadAndId(state, resolvedThreadId, toolCallId),
      ) ||
      !(
        selectIsStreamingById(state, resolvedThreadId) ||
        selectIsWaitingById(state, resolvedThreadId)
      )
    ) {
      return false;
    }
    const messages = selectVisibleMessages(state, resolvedThreadId);
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index];
      if (message.role === "user") return false;
      if (message.role === "assistant") {
        return (
          message.tool_calls?.some((call) => call.id === toolCallId) === true
        );
      }
    }
    return false;
  });
  return { result, isRunning, threadId: resolvedThreadId };
}
