import type { RootState } from "../../app/store";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useThreadId } from "../../features/Chat/Thread";
import { selectMessagesById } from "../../features/Chat/Thread/selectors";
import { normalizeToolName } from "../../utils/toolNameAliases";

const FINAL_REPORT_TOOLS = new Set(["task_done", "agent_finish"]);

export function selectLatestFinalReportId(
  state: RootState,
  threadId: string,
): string | null {
  const messages = selectMessagesById(state, threadId);
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index];
    if (message.role !== "assistant" || !message.tool_calls) continue;
    for (let call = message.tool_calls.length - 1; call >= 0; call--) {
      const toolCall = message.tool_calls[call];
      const name = normalizeToolName(toolCall.function.name);
      if (name && FINAL_REPORT_TOOLS.has(name)) return toolCall.id ?? null;
    }
  }
  return null;
}

export function useIsLatestFinalReport(toolCallId: string | undefined) {
  const threadId = useThreadId();
  const latestId = useAppSelector((state) =>
    selectLatestFinalReportId(state, threadId),
  );
  return toolCallId !== undefined && latestId === toolCallId;
}
