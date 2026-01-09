import React, { useMemo, useEffect, useRef, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";

import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectMessages,
  selectThreadMaximumTokens,
} from "../../features/Chat";
import { AssistantMessage, isAssistantMessage } from "../../services/refact";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";

import styles from "./StreamingTokenCounter.module.css";

function extractAllText(msg: AssistantMessage | null): string {
  if (!msg) return "";
  const parts: string[] = [];

  if (typeof msg.content === "string") parts.push(msg.content);
  if (typeof msg.reasoning_content === "string") parts.push(msg.reasoning_content);

  if (Array.isArray(msg.thinking_blocks)) {
    for (const block of msg.thinking_blocks) {
      if (typeof block.thinking === "string") {
        parts.push(block.thinking);
      }
    }
  }

  return parts.join("");
}

function estimateTokens(text: string): number {
  if (!text) return 0;
  return Math.ceil(text.length / 4);
}

export const StreamingTokenCounter: React.FC = () => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectThreadMaximumTokens) ?? 0;

  const [displayTokens, setDisplayTokens] = useState(0);
  const [pulseKey, setPulseKey] = useState(0);
  const prevTokensRef = useRef(0);

  const lastAssistantMessage = useMemo((): AssistantMessage | null => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const msg = messages[i];
      if (isAssistantMessage(msg)) {
        return msg;
      }
    }
    return null;
  }, [messages]);

  const usage = lastAssistantMessage?.usage;
  const allText = useMemo(() => extractAllText(lastAssistantMessage), [lastAssistantMessage]);

  const actualOutputTokens = usage?.completion_tokens ?? 0;
  const estimatedOutputTokens = useMemo(() => {
    return estimateTokens(allText);
  }, [allText]);

  const outputTokens = actualOutputTokens > 0 ? actualOutputTokens : estimatedOutputTokens;
  const contextTokens = usage?.prompt_tokens ?? 0;

  const contextPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return Math.round((contextTokens / maxContextTokens) * 100);
  }, [contextTokens, maxContextTokens]);

  useEffect(() => {
    if (outputTokens !== prevTokensRef.current) {
      prevTokensRef.current = outputTokens;
      setDisplayTokens(outputTokens);
      setPulseKey((k) => k + 1);
    }
  }, [outputTokens]);

  useEffect(() => {
    if (!isStreaming) {
      setDisplayTokens(0);
      prevTokensRef.current = 0;
      setPulseKey(0);
    }
  }, [isStreaming]);

  if (!isStreaming) return null;

  const isEstimate = actualOutputTokens === 0;

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
          : `${isEstimate ? "~" : ""}${formatNumberToFixed(displayTokens)}`}
      </Text>

      {contextTokens > 0 && maxContextTokens > 0 && (
        <Text
          className={classNames(styles.contextPercent, {
            [styles.warning]: contextPercentage >= 70,
            [styles.critical]: contextPercentage >= 90,
          })}
        >
          ({contextPercentage}%)
        </Text>
      )}
    </Flex>
  );
};
