import { useMemo } from "react";
import {
  selectMessagesById,
  selectEffectiveMaxContextTokensById,
  useThreadId,
} from "../../features/Chat/Thread";
import { useAppSelector } from "../../hooks";
import {
  calculateUsageInputTokens,
  getCacheCreationTokens,
  getCacheReadTokens,
  mergeUsages,
} from "../../utils/calculateUsageInputTokens";
import { isAssistantMessage } from "../../services/refact";

export function useUsageCounter() {
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  const maxContextTokens = useAppSelector((state) =>
    selectEffectiveMaxContextTokensById(state, chatId),
  );

  const { assistantMessages, currentThreadUsage, lastAssistantMessage } =
    useMemo(() => {
      const assistants = messages.filter(isAssistantMessage);
      const mergedUsage = mergeUsages(assistants.map((msg) => msg.usage));
      const lastAssistant =
        assistants.length > 0 ? assistants[assistants.length - 1] : undefined;

      return {
        assistantMessages: assistants,
        currentThreadUsage: mergedUsage,
        lastAssistantMessage: lastAssistant,
      };
    }, [messages]);

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
    for (let i = assistantMessages.length - 1; i >= 0; i--) {
      const usage = assistantMessages[i]?.usage;
      if (!usage) continue;
      const promptTokens = usage.prompt_tokens;
      const cacheCreation = getCacheCreationTokens(usage);
      const cacheRead = getCacheReadTokens(usage);
      const total = promptTokens + cacheCreation + cacheRead;
      if (total > 0) return total;
    }
    return 0;
  }, [assistantMessages]);

  const isContextFromPreviousMessage = useMemo(() => {
    if (assistantMessages.length === 0) return false;
    const lastMsg = assistantMessages[assistantMessages.length - 1];
    const usage = lastMsg.usage;
    const lastTotal =
      (usage?.prompt_tokens ?? 0) +
      getCacheCreationTokens(usage) +
      getCacheReadTokens(usage);
    return lastTotal === 0 && currentSessionTokens > 0;
  }, [assistantMessages, currentSessionTokens]);

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
