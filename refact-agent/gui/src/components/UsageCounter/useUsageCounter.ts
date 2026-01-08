import { useMemo, useRef } from "react";
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
  const assistantMessages = messages.filter(isAssistantMessage);
  const usages = assistantMessages.map((msg) => msg.usage);
  const currentThreadUsage = mergeUsages(usages);
  const lastAssistantMessage =
    assistantMessages.length > 0
      ? assistantMessages[assistantMessages.length - 1]
      : undefined;
  const lastUsage = lastAssistantMessage?.usage;

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

  const lastKnownTokensRef = useRef(0);
  const rawTokens = lastUsage?.prompt_tokens ?? 0;
  if (rawTokens > 0) {
    lastKnownTokensRef.current = rawTokens;
  }
  const currentSessionTokens = rawTokens > 0 ? rawTokens : lastKnownTokensRef.current;

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

  const isContextFull = useMemo(() => {
    return tokenPercentage >= 97;
  }, [tokenPercentage]);

  return {
    shouldShow,
    currentThreadUsage,
    totalInputTokens,
    currentSessionTokens,
    isOverflown,
    isWarning,
    isContextFull,
    tokenPercentage,
  };
}
