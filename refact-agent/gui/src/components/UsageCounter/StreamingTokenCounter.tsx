import React, { useMemo, useEffect, useRef, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";

import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectMessages,
  selectThreadMaximumTokens,
  selectStreamVersion,
} from "../../features/Chat";
import { AssistantMessage, isAssistantMessage } from "../../services/refact";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";

import styles from "./StreamingTokenCounter.module.css";

/**
 * Estimate token count from text content.
 * Uses a simple heuristic: ~4 characters per token (common for English text).
 * This is an approximation - actual tokenization varies by model.
 */
function estimateTokens(text: string): number {
  if (!text) return 0;
  // Rough estimate: 1 token ≈ 4 characters for English
  // This is a common approximation used by many tools
  return Math.ceil(text.length / 4);
}

/**
 * StreamingTokenCounter - Compact live token counter for use inside Stop button
 *
 * Shows estimated output tokens during streaming based on content length.
 * Once streaming completes, shows actual token count from API if available.
 *
 * Note: Most providers (OpenAI, Anthropic) only send usage data at stream END.
 * xAI/Grok sends incremental usage. We estimate tokens during streaming for
 * providers that don't support incremental usage reporting.
 */
export const StreamingTokenCounter: React.FC = () => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectThreadMaximumTokens) ?? 0;
  // Subscribe to stream_version to force re-renders on every delta
  // The value itself is not used, but subscribing triggers re-renders
  void useAppSelector(selectStreamVersion);

  // Track for animation
  const [displayTokens, setDisplayTokens] = useState(0);
  const prevTokensRef = useRef(0);

  // Get the last assistant message (the one being streamed)
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
  const content = lastAssistantMessage?.content ?? "";

  // Actual output tokens from API (if available)
  const actualOutputTokens = usage?.completion_tokens ?? 0;

  // Estimated tokens from content (for live display during streaming)
  const estimatedOutputTokens = useMemo(() => {
    return estimateTokens(content);
  }, [content]);

  // Use actual tokens if available, otherwise use estimate
  // During streaming, providers usually don't send usage until the end
  const outputTokens = actualOutputTokens > 0 ? actualOutputTokens : estimatedOutputTokens;

  // Context tokens (prompt_tokens) - usually available at start for some providers
  const contextTokens = usage?.prompt_tokens ?? 0;

  // Context percentage
  const contextPercentage = useMemo(() => {
    if (!maxContextTokens || maxContextTokens === 0) return 0;
    return Math.round((contextTokens / maxContextTokens) * 100);
  }, [contextTokens, maxContextTokens]);

  // Update display with animation when tokens change
  useEffect(() => {
    if (outputTokens !== prevTokensRef.current) {
      prevTokensRef.current = outputTokens;
      setDisplayTokens(outputTokens);
    }
  }, [outputTokens]);

  // Reset when streaming stops
  useEffect(() => {
    if (!isStreaming) {
      setDisplayTokens(0);
      prevTokensRef.current = 0;
    }
  }, [isStreaming]);

  // Don't show anything if no content yet
  if (!isStreaming || displayTokens === 0) return null;

  // Show "~" prefix when using estimates (no actual usage data yet)
  const isEstimate = actualOutputTokens === 0;

  return (
    <Flex align="center" gap="1" className={styles.inlineContainer}>
      {/* Separator */}
      <Text className={styles.separator}>|</Text>

      {/* Output tokens (live counter) */}
      <Text
        className={classNames(styles.tokenValue, {
          [styles.animateValue]: displayTokens > 0,
        })}
      >
        {isEstimate ? "~" : ""}
        {formatNumberToFixed(displayTokens)}
      </Text>

      {/* Context percentage if available */}
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
