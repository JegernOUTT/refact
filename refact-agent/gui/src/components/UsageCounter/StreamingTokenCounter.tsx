import React, { useMemo, useEffect, useRef, useState } from "react";
import { Flex, Text } from "../LongTailPrimitives";
import { ArrowDownIcon } from "@radix-ui/react-icons";
import classNames from "classnames";

import { useAppSelector } from "../../hooks";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  useThreadId,
} from "../../features/Chat/Thread";
import {
  AssistantMessage,
  isAssistantMessage,
  isUserMessage,
} from "../../services/refact";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";

import styles from "./StreamingTokenCounter.module.css";

function estimateTokensFromLength(length: number): number {
  if (length <= 0) return 0;
  return Math.ceil(length / 4);
}

function findLastIndex<T>(arr: T[], pred: (x: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (pred(arr[i])) return i;
  }
  return -1;
}

function getTextLength(message: AssistantMessage | null): number {
  if (!message) return 0;

  let len = message.content?.length ?? 0;

  if (message.reasoning_content) {
    len += message.reasoning_content.length;
  }

  if (message.thinking_blocks) {
    for (const block of message.thinking_blocks) {
      if (block.thinking) len += block.thinking.length;
      if (block.signature) len += block.signature.length;
    }
  }

  return len;
}

export const StreamingTokenCounter: React.FC = () => {
  const chatId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));

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

  const activeAssistantMessage = useMemo((): AssistantMessage | null => {
    if (waitingForNewAssistant) return null;
    if (lastAssistantIdx < 0) return null;
    const msg = messages[lastAssistantIdx];
    return isAssistantMessage(msg) ? msg : null;
  }, [messages, lastAssistantIdx, waitingForNewAssistant]);

  const usage = activeAssistantMessage?.usage;

  const textLength = useMemo(
    (): number => getTextLength(activeAssistantMessage),
    [activeAssistantMessage],
  );

  const actualOutputTokens = usage?.completion_tokens ?? 0;
  const estimatedOutputTokens = useMemo((): number => {
    return estimateTokensFromLength(textLength);
  }, [textLength]);

  const outputTokens: number =
    actualOutputTokens > 0 ? actualOutputTokens : estimatedOutputTokens;

  const hasAnyOutput = textLength > 0 || outputTokens > 0;
  const hasFinalUsage =
    (usage?.prompt_tokens ?? 0) > 0 || (usage?.completion_tokens ?? 0) > 0;

  useEffect(() => {
    if (hideTimerRef.current) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }

    if (isStreaming || isWaiting) {
      setVisible(true);
    } else if (hasAnyOutput && !hasFinalUsage) {
      setVisible(true);
      hideTimerRef.current = window.setTimeout(() => setVisible(false), 60_000);
    } else if (hasFinalUsage) {
      setVisible(true);
      hideTimerRef.current = window.setTimeout(() => setVisible(false), 2_000);
    } else {
      setVisible(false);
    }

    return () => {
      if (hideTimerRef.current) {
        window.clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
    };
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

  const hasNoOutput = textLength === 0 && outputTokens === 0;
  if (hasNoOutput) return null;

  const isOutputEstimate = actualOutputTokens === 0;

  const tokensToDisplay =
    isStreaming || isWaiting ? outputTokens : displayTokens;

  return (
    <Flex align="center" gap="1" className={styles.inlineContainer}>
      <Text
        key={pulseKey}
        size="1"
        color="gray"
        className={classNames(styles.tokenValue, {
          [styles.animateValue]: tokensToDisplay > 0,
        })}
      >
        {isOutputEstimate ? "~" : ""}
        {formatNumberToFixed(tokensToDisplay)}
      </Text>
      <ArrowDownIcon width={12} height={12} />
    </Flex>
  );
};
