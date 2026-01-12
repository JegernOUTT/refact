import { useMemo } from "react";
import { selectMessages, selectThreadMaximumTokens } from "../../features/Chat";
import { useAppSelector } from "../../hooks";
import {
  calculateUsageInputTokens,
  mergeUsages,
} from "../../utils/calculateUsageInputTokens";
import { isAssistantMessage } from "../../services/refact";

export function useUsageCounter() {
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectThreadMaximumTokens);

  // Memoize assistant messages list
  const assistantMessages = useMemo(
    () => messages.filter(isAssistantMessage),
    [messages],
  );

  // Memoize usages list
  const usages = useMemo(
    () => assistantMessages.map((msg) => msg.usage),
    [assistantMessages],
  );

  const currentThreadUsage = mergeUsages(usages);

  const lastAssistantMessage = useMemo(
    () =>
      assistantMessages.length > 0
        ? assistantMessages[assistantMessages.length - 1]
        : undefined,
    [assistantMessages],
  );

  // Check if the last message has server-executed tools (like web_search)
  // These can cause temporary inflated token counts during streaming.
  // We check both server_executed_tools (set after streaming) and tool_calls
  // with srvtoolu_ prefix (visible during streaming)
  const hasServerExecutedTools = useMemo(() => {
    if (!lastAssistantMessage) return false;
    // Check post-processed server_executed_tools
    const serverTools = lastAssistantMessage.server_executed_tools;
    if (Array.isArray(serverTools) && serverTools.length > 0) {
      return true;
    }
    // Check tool_calls during streaming (before post-processing)
    const toolCalls = lastAssistantMessage.tool_calls;
    if (Array.isArray(toolCalls)) {
      return toolCalls.some((tc) => tc.id?.startsWith("srvtoolu_"));
    }
    return false;
  }, [lastAssistantMessage]);

  const totalInputTokens = useMemo(() => {
    return calculateUsageInputTokens({
      usage: currentThreadUsage,
      keys: [
        "prompt_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
      ],
    });
  }, [currentThreadUsage]);

  // Deterministic fallback: scan backwards through assistant messages for first message with prompt_tokens > 0
  const currentSessionTokens = useMemo(() => {
    for (let i = assistantMessages.length - 1; i >= 0; i--) {
      const t = assistantMessages[i]?.usage?.prompt_tokens;
      if (typeof t === "number" && t > 0) return t;
    }
    return 0;
  }, [assistantMessages]);

  const isContextFromPreviousMessage = useMemo(() => {
    if (assistantMessages.length === 0) return false;
    const lastMsg = assistantMessages[assistantMessages.length - 1];
    const lastHasTokens = (lastMsg.usage?.prompt_tokens ?? 0) > 0;
    return !lastHasTokens && currentSessionTokens > 0;
  }, [assistantMessages, currentSessionTokens]);

  const tokenPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return (currentSessionTokens / maxContextTokens) * 100;
  }, [currentSessionTokens, maxContextTokens]);

  const isWarning = useMemo(() => {
    return tokenPercentage >= 85;
  }, [tokenPercentage]);

  const isOverflown = useMemo(() => {
    return tokenPercentage >= 97;
  }, [tokenPercentage]);

  const shouldShow = useMemo(() => {
    return messages.length > 0;
  }, [messages.length]);

  // Don't mark context as full when server-executed tools are present
  // Claude's web_search can report inflated token counts during streaming
  // that normalize after completion - this prevents false blocking
  const isContextFull = useMemo(() => {
    if (hasServerExecutedTools) return false;
    return tokenPercentage >= 97;
  }, [tokenPercentage, hasServerExecutedTools]);

  return {
    shouldShow,
    currentThreadUsage,
    totalInputTokens,
    currentSessionTokens,
    isOverflown,
    isWarning,
    isContextFull,
    tokenPercentage,
    hasServerExecutedTools,
    isContextFromPreviousMessage,
  };
}
