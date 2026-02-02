import React, { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Flex, Text, Spinner } from "@radix-ui/themes";
import { LightningBoltIcon } from "@radix-ui/react-icons";

import { Markdown } from "../../Markdown";

import styles from "./ReasoningContent.module.css";

type ReasoningContentProps = {
  reasoningContent: string;
  onCopyClick: (text: string) => void;
  isStreaming?: boolean;
  hasMessageContent?: boolean;
};

function formatDuration(seconds: number): string {
  if (seconds < 60) {
    return `${Math.round(seconds)} seconds`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.round(seconds % 60);
  if (remainingSeconds === 0) {
    return `${minutes} minute${minutes > 1 ? "s" : ""}`;
  }
  return `${minutes}m ${remainingSeconds}s`;
}

export const ReasoningContent: React.FC<ReasoningContentProps> = ({
  reasoningContent,
  onCopyClick,
  isStreaming = false,
  hasMessageContent = false,
}) => {
  const [isOpen, setIsOpen] = useState(true);
  const [thinkingDuration, setThinkingDuration] = useState<number | null>(null);
  const startTimeRef = useRef<number | null>(null);
  const userToggledRef = useRef(false);
  const wasThinkingRef = useRef(false);
  const durationCapturedRef = useRef(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);

  // Track thinking duration - stop when message content starts appearing
  useEffect(() => {
    const isActivelyThinking =
      isStreaming && !!reasoningContent && !hasMessageContent;

    if (isActivelyThinking) {
      // Started thinking
      if (startTimeRef.current === null) {
        startTimeRef.current = Date.now();
      }
      wasThinkingRef.current = true;
    } else if (
      wasThinkingRef.current &&
      startTimeRef.current !== null &&
      !durationCapturedRef.current
    ) {
      // Thinking finished (message content started or streaming ended)
      const duration = (Date.now() - startTimeRef.current) / 1000;
      setThinkingDuration(duration);
      durationCapturedRef.current = true;
    }
  }, [isStreaming, reasoningContent, hasMessageContent]);

  // Auto-collapse after entire message finishes streaming
  useEffect(() => {
    if (!isStreaming && wasThinkingRef.current && !userToggledRef.current) {
      const timer = setTimeout(() => {
        setIsOpen(false);
      }, 500);
      return () => clearTimeout(timer);
    }
  }, [isStreaming]);

  // Handle initial mount for already-completed thinking blocks
  useEffect(() => {
    if (
      !isStreaming &&
      reasoningContent &&
      thinkingDuration === null &&
      startTimeRef.current === null
    ) {
      // This is a historical thinking block (page reload or switching chats)
      // Start collapsed since we don't have timing info
      setIsOpen(false);
    }
  }, [isStreaming, reasoningContent, thinkingDuration]);

  // Auto-scroll to bottom while streaming
  useEffect(() => {
    if (
      isStreaming &&
      isOpen &&
      contentRef.current &&
      !userScrolledRef.current
    ) {
      contentRef.current.scrollTop = contentRef.current.scrollHeight;
    }
  }, [reasoningContent, isStreaming, isOpen]);

  // Reset user scroll flag when streaming starts
  useEffect(() => {
    if (isStreaming) {
      userScrolledRef.current = false;
    }
  }, [isStreaming]);

  // Handle user scroll to disable auto-scroll
  const handleScroll = useCallback(() => {
    if (contentRef.current && isStreaming) {
      const { scrollTop, scrollHeight, clientHeight } = contentRef.current;
      // If user scrolled up (not at bottom), disable auto-scroll
      const isAtBottom = scrollHeight - scrollTop - clientHeight < 20;
      if (!isAtBottom) {
        userScrolledRef.current = true;
      }
    }
  }, [isStreaming]);

  const handleToggle = useCallback(() => {
    userToggledRef.current = true;
    setIsOpen((prev) => !prev);
  }, []);

  const isActivelyThinking =
    isStreaming && !!reasoningContent && !hasMessageContent;
  const summaryText = isActivelyThinking
    ? "Thinking..."
    : thinkingDuration !== null
      ? `Thought for ${formatDuration(thinkingDuration)}`
      : "Thought";

  return (
    <div className={styles.card}>
      <Flex
        className={`${styles.header} ${
          isActivelyThinking ? styles.thinking : ""
        }`}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <span className={styles.iconWrapper}>
          {isActivelyThinking ? <Spinner size="1" /> : <LightningBoltIcon />}
        </span>
        <Text size="1" className={styles.summary}>
          {summaryText}
        </Text>
      </Flex>

      <AnimatePresence initial={false}>
        {isOpen && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: "easeInOut" }}
            className={styles.contentWrapper}
          >
            <div
              ref={contentRef}
              className={styles.content}
              onScroll={handleScroll}
            >
              <Text size="2" color="gray">
                <Markdown
                  canHaveInteractiveElements={true}
                  onCopyClick={onCopyClick}
                >
                  {reasoningContent}
                </Markdown>
              </Text>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};
