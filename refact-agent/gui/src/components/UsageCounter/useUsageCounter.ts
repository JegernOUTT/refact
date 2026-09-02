import { useMemo } from "react";
import {
  selectEffectiveMaxContextTokensById,
  selectLastAssistantMessageById,
  selectMessagesCountById,
  useThreadId,
} from "../../features/Chat/Thread";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  calculateUsageInputTokens,
  getCacheCreationTokens,
  getCacheReadTokens,
  mergeUsages,
} from "../../utils/calculateUsageInputTokens";

export function useUsageCounter() {
  const chatId = useThreadId();
  const maxContextTokens = useAppSelector((state) =>
    selectEffectiveMaxContextTokensById(state, chatId),
  );
  const messageCount = useAppSelector((state) =>
    selectMessagesCountById(state, chatId),
  );
  const lastAssistantMessage = useAppSelector((state) =>
    selectLastAssistantMessageById(state, chatId),
  );

  const currentThreadUsage = useMemo(
    () => mergeUsages(lastAssistantMessage ? [lastAssistantMessage.usage] : []),
    [lastAssistantMessage],
  );

  // Check if the last message has server-executed tools (like web_search)
  // These can cause temporary inflated token counts during streaming.
  // We check both server_executed_tools (set after streaming) and tool_calls
  // with srvtoolu_ prefix (visible during streaming)
  const hasServerExecutedTools = useMemo(() => {
    if (!lastAssistantMessage) return false;
    const serverTools = lastAssistantMessage.server_executed_tools;
    if (Array.isArray(serverTools) && serverTools.length > 0) {
      return true;
    }
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

  // Deterministic fallback: scan backwards through assistant messages for first message with input tokens > 0
  // Include cache tokens for accurate context size (prompt_tokens + cache_creation + cache_read)
  const currentSessionTokens = useMemo(() => {
    const usage = lastAssistantMessage?.usage;
    if (!usage) return 0;
    return (
      usage.prompt_tokens +
      getCacheCreationTokens(usage) +
      getCacheReadTokens(usage)
    );
  }, [lastAssistantMessage]);

  const isContextFromPreviousMessage = useMemo(() => {
    if (!lastAssistantMessage) return false;
    const usage = lastAssistantMessage.usage;
    const lastTotal =
      (usage?.prompt_tokens ?? 0) +
      getCacheCreationTokens(usage) +
      getCacheReadTokens(usage);
    return lastTotal === 0 && currentSessionTokens > 0;
  }, [lastAssistantMessage, currentSessionTokens]);

  const tokenPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return (currentSessionTokens / maxContextTokens) * 100;
  }, [currentSessionTokens, maxContextTokens]);

  // Don't show warnings when server-executed tools are present
  // Claude's web_search can report inflated token counts during streaming
  // that normalize after completion - this prevents false warnings
  const isWarning = useMemo(() => {
    if (hasServerExecutedTools) return false;
    return tokenPercentage >= 85;
  }, [tokenPercentage, hasServerExecutedTools]);

  const isOverflown = useMemo(() => {
    if (hasServerExecutedTools) return false;
    return tokenPercentage >= 97;
  }, [tokenPercentage, hasServerExecutedTools]);

  const shouldShow = useMemo(() => {
    return messageCount > 0;
  }, [messageCount]);

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
