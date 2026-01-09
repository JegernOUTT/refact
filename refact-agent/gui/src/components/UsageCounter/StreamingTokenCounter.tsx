import React, { useMemo, useEffect, useRef, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";

import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectThreadMaximumTokens,
} from "../../features/Chat";
import { AssistantMessage, isAssistantMessage, ChatMessage } from "../../services/refact";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import { useUsageCounter } from "./useUsageCounter";

import styles from "./StreamingTokenCounter.module.css";

function estimateTokens(text: string): number {
  if (!text) return 0;
  return Math.ceil(text.length / 4);
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

function findLastAssistantMessage(messages: ChatMessage[]): AssistantMessage | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (isAssistantMessage(msg)) return msg;
  }
  return null;
}

function findLastNonZeroPromptTokens(messages: ChatMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (!isAssistantMessage(msg)) continue;
    const t = msg.usage?.prompt_tokens;
    if (typeof t === "number" && t > 0) return t;
  }
  return 0;
}

export const StreamingTokenCounter: React.FC = () => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectThreadMaximumTokens) ?? 0;
  
  const { isContextFromPreviousMessage } = useUsageCounter();

  const lastAssistantMessage = useMemo(
    () => findLastAssistantMessage(messages),
    [messages],
  );

  const usage = lastAssistantMessage?.usage;
  const allText = useMemo(() => extractAllText(lastAssistantMessage), [lastAssistantMessage]);

  const actualOutputTokens = usage?.completion_tokens ?? 0;
  const estimatedOutputTokens = useMemo(() => {
    return estimateTokens(allText);
  }, [allText]);

  const outputTokens = actualOutputTokens > 0 ? actualOutputTokens : estimatedOutputTokens;

  const actualContextTokens = usage?.prompt_tokens ?? 0;
  const fallbackContextTokens = useMemo(
    () => findLastNonZeroPromptTokens(messages),
    [messages],
  );
  const contextTokens =
    actualContextTokens > 0 ? actualContextTokens : fallbackContextTokens;

  const [displayTokens, setDisplayTokens] = useState(0);
  const [pulseKey, setPulseKey] = useState(0);
  const prevTokensRef = useRef(0);

  useEffect(() => {
    if (outputTokens !== prevTokensRef.current) {
      prevTokensRef.current = outputTokens;
      setDisplayTokens(outputTokens);
      setPulseKey((k) => k + 1);
    }
  }, [outputTokens]);

  const [visible, setVisible] = useState(false);
  const hideTimerRef = useRef<number | null>(null);

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
    if (!visible) {
      setDisplayTokens(0);
      prevTokensRef.current = 0;
      setPulseKey(0);
    }
  }, [visible]);

  const contextPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return Math.round((contextTokens / maxContextTokens) * 100);
  }, [contextTokens, maxContextTokens]);

  const isOutputEstimate = actualOutputTokens === 0;

  if (!visible || !lastAssistantMessage) return null;

  return (
    <Flex align="center" gap="1" className={styles.inlineContainer}>
      <Text className={styles.separator}>|</Text>

      <Text
        key={pulseKey}
        className={classNames(styles.tokenValue, {
          [styles.animateValue]: displayTokens > 0,
        })}
      >
        {displayTokens === 0 && allText.length === 0
          ? "…"
          : `${isOutputEstimate ? "~" : ""}${formatNumberToFixed(displayTokens)}`}
      </Text>

      {contextTokens > 0 && maxContextTokens > 0 && (
        <Text
          className={classNames(styles.contextPercent, {
            [styles.warning]: contextPercentage >= 70 && !isContextFromPreviousMessage,
            [styles.critical]: contextPercentage >= 90 && !isContextFromPreviousMessage,
            [styles.fallback]: isContextFromPreviousMessage,
          })}
        >
          ({isContextFromPreviousMessage ? "~" : ""}{contextPercentage}%)
        </Text>
      )}
    </Flex>
  );
};
