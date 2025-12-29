import { useMemo, useRef } from "react";
import { selectMessages } from "../../features/Chat";
import { useAppSelector, useLastSentCompressionStop } from "../../hooks";
import {
  calculateUsageInputTokens,
  mergeUsages,
} from "../../utils/calculateUsageInputTokens";
import { isAssistantMessage } from "../../services/refact";

export function useUsageCounter() {
  const compressionStop = useLastSentCompressionStop();
  const messages = useAppSelector(selectMessages);
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

  const isOverflown = useMemo(() => {
    if (compressionStop.strength === "low") return true;
    if (compressionStop.strength === "medium") return true;
    if (compressionStop.strength === "high") return true;
    return false;
  }, [compressionStop.strength]);

  const isWarning = useMemo(() => {
    if (compressionStop.strength === "medium") return true;
    if (compressionStop.strength === "high") return true;
    return false;
  }, [compressionStop.strength]);

  const shouldShow = useMemo(() => {
    return messages.length > 0;
  }, [messages.length]);

  return {
    shouldShow,
    currentThreadUsage,
    totalInputTokens,
    currentSessionTokens,
    isOverflown,
    isWarning,
    compressionStrength: compressionStop.strength,
  };
}
