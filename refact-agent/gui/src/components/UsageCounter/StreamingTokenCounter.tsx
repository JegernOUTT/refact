import React, { useMemo, useEffect, useRef, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";

import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectThreadMaximumTokens,
  selectStreamVersion,
} from "../../features/Chat";
import {
  AssistantMessage,
  isAssistantMessage,
  isUserMessage,
} from "../../services/refact";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import { useUsageCounter } from "./useUsageCounter";

import styles from "./StreamingTokenCounter.module.css";

function estimateTokens(text: string): number {
  if (!text) return 0;
  return Math.ceil(text.length / 4);
}

function findLastIndex<T>(arr: T[], pred: (x: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (pred(arr[i])) return i;
  }
  return -1;
}

function extractAllText(message: AssistantMessage | null): string {
  if (!message) return "";

  let text = message.content ?? "";

  if (message.reasoning_content) {
    text += message.reasoning_content;
  }

  if (message.thinking_blocks) {
    for (const block of message.thinking_blocks) {
      if (block.thinking) text += block.thinking;
      if (block.signature) text += block.signature;
    }
  }

  return text;
}

export const StreamingTokenCounter: React.FC = () => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectThreadMaximumTokens) ?? 0;
  // Subscribe to stream_version to force re-renders on every delta
  // The value itself is not used, but subscribing triggers re-renders
  void useAppSelector(selectStreamVersion);

  const { currentSessionTokens, isContextFromPreviousMessage } = useUsageCounter();

  const [visible, setVisible] = useState(() => isStreaming || isWaiting);
  const [displayTokens, setDisplayTokens] = useState(0);
  const [pulseKey, setPulseKey] = useState(0);
  const prevTokensRef = useRef(0);
  const hideTimerRef = useRef<number | null>(null);

  const lastAssistantIdx = useMemo(
    () => findLastIndex(messages, isAssistantMessage),
    [messages],
  );
  const lastUserIdx = useMemo(
    () => findLastIndex(messages, isUserMessage),
    [messages],
  );

  const waitingForNewAssistant =
    (isWaiting || isStreaming) && lastUserIdx > lastAssistantIdx;

  const activeAssistantMessage = useMemo(
    (): AssistantMessage | null => {
      if (waitingForNewAssistant) return null;
      if (lastAssistantIdx < 0) return null;
      const msg = messages[lastAssistantIdx];
      return isAssistantMessage(msg) ? msg : null;
    },
    [messages, lastAssistantIdx, waitingForNewAssistant],
  );

  const usage = activeAssistantMessage?.usage;

  const allText = useMemo(
    (): string => extractAllText(activeAssistantMessage),
    [activeAssistantMessage],
  );

  const actualOutputTokens = usage?.completion_tokens ?? 0;
  const estimatedOutputTokens = useMemo((): number => {
    return estimateTokens(allText);
  }, [allText]);

  const outputTokens: number =
    actualOutputTokens > 0 ? actualOutputTokens : estimatedOutputTokens;

  const actualContextTokens = usage?.prompt_tokens ?? 0;
  const contextTokens =
    actualContextTokens > 0 ? actualContextTokens : currentSessionTokens;

  const contextPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return Math.round((contextTokens / maxContextTokens) * 100);
  }, [contextTokens, maxContextTokens]);

  const hasAnyOutput = allText.length > 0 || outputTokens > 0;
  const hasFinalUsage = (usage?.prompt_tokens ?? 0) > 0 || (usage?.completion_tokens ?? 0) > 0;

  useEffect(() => {
    if (hideTimerRef.current) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }

    if (isStreaming || isWaiting) {
      setVisible(true);
      return;
    }

    if (hasAnyOutput && !hasFinalUsage) {
      setVisible(true);
      hideTimerRef.current = window.setTimeout(() => setVisible(false), 60_000);
      return;
    }

    if (hasFinalUsage) {
      setVisible(true);
      hideTimerRef.current = window.setTimeout(() => setVisible(false), 2_000);
      return;
    }

    setVisible(false);
  }, [isStreaming, isWaiting, hasAnyOutput, hasFinalUsage]);

  useEffect(() => {
    if (outputTokens !== prevTokensRef.current) {
      prevTokensRef.current = outputTokens;
      setDisplayTokens(outputTokens);
      setPulseKey((k: number) => k + 1);
    }
  }, [outputTokens]);

  useEffect(() => {
    if (!visible) {
      setDisplayTokens(0);
      prevTokensRef.current = 0;
      setPulseKey(0);
    }
  }, [visible]);

  if (!visible) return null;

  const showPlaceholder = allText.length === 0 && (isStreaming || isWaiting);
  const isOutputEstimate = actualOutputTokens === 0;

  const tokensToDisplay = isStreaming || isWaiting ? outputTokens : displayTokens;

  return (
    <Flex align="center" gap="1" className={styles.inlineContainer}>
      <Text className={styles.separator}>|</Text>

      <Text
        key={pulseKey}
        className={classNames(styles.tokenValue, {
          [styles.animateValue]: tokensToDisplay > 0,
        })}
      >
        {showPlaceholder
          ? "…"
          : `${isOutputEstimate ? "~" : ""}${formatNumberToFixed(tokensToDisplay)}`}
      </Text>

      {contextTokens > 0 && maxContextTokens > 0 && (
        <Text
          className={classNames(styles.contextPercent, {
            [styles.fallback]: isContextFromPreviousMessage,
            [styles.warning]: contextPercentage >= 70 && !isContextFromPreviousMessage,
            [styles.critical]: contextPercentage >= 90 && !isContextFromPreviousMessage,
          })}
        >
          ({isOutputEstimate || isContextFromPreviousMessage ? "~" : ""}
          {contextPercentage}%)
        </Text>
      )}
    </Flex>
  );
};
