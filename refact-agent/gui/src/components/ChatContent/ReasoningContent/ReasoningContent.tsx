import React, {
  useState,
  useEffect,
  useRef,
  useCallback,
  useMemo,
} from "react";
import classNames from "classnames";
import { LoaderCircle, Zap } from "lucide-react";
import { Icon } from "../../ui";

import { Markdown } from "../../Markdown";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";

import { useCollapsibleStore } from "../useStoredOpen";
import styles from "./ReasoningContent.module.css";

// Bold titles like "**Some Title**" often appear glued to the end of the
// previous sentence in reasoning summaries.  Insert a paragraph break before
// them so Markdown renders each title as a separate block.
// The regex matches "**" preceded by a non-whitespace char where the first
// letter after "**" is uppercase — so inline bold like "**sorted**" is left
// alone.
function fixReasoningParagraphs(text: string): string {
  return text.replace(/(\S)(\*\*[A-Z])/g, "$1\n\n$2");
}

type ReasoningContentProps = {
  reasoningContent: string;
  onCopyClick: (text: string) => void;
  isStreaming?: boolean;
  hasMessageContent?: boolean;
  stateKey?: string;
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
  stateKey,
}) => {
  const store = useCollapsibleStore();
  const storedOpen = stateKey && store ? store.get(stateKey) : undefined;
  const hadStoredStateRef = useRef(storedOpen !== undefined);

  const [isOpen, setIsOpen] = useState(storedOpen ?? true);
  const [thinkingDuration, setThinkingDuration] = useState<number | null>(null);
  const startTimeRef = useRef<number | null>(null);
  const userToggledRef = useRef(false);
  const wasThinkingRef = useRef(false);
  const durationCapturedRef = useRef(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);

  useEffect(() => {
    if (stateKey && store) store.set(stateKey, isOpen);
  }, [stateKey, store, isOpen]);

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
    if (hadStoredStateRef.current) return;
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
      const el = contentRef.current;
      if (el.scrollTop + el.clientHeight + 20 < el.scrollHeight) {
        el.scrollTop = el.scrollHeight;
      }
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

  const formattedContent = useMemo(
    () => fixReasoningParagraphs(reasoningContent),
    [reasoningContent],
  );
  const deferredContent = useStreamingMarkdown(
    formattedContent,
    isStreaming && isOpen,
  );

  useEffect(() => {
    if (isStreaming && isOpen) {
      const text = deferredContent ?? formattedContent;
      setBuddyCrashHotSlot("reasoning", text);
      addBuddyCrashBreadcrumb("reasoning", text);
      return;
    }

    setBuddyCrashHotSlot("reasoning", null);
  }, [deferredContent, formattedContent, isOpen, isStreaming]);

  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);

  return (
    <div className={styles.card}>
      <div
        className={classNames(
          styles.header,
          isActivelyThinking && "rf-active-pulse",
          isActivelyThinking && styles.thinking,
        )}
        onClick={handleToggle}
      >
        <span className={styles.iconWrapper}>
          {isActivelyThinking ? (
            <span className={styles.spinner}><Icon icon={LoaderCircle} size="sm" tone="accent" /></span>
          ) : (
            <Icon icon={Zap} size="sm" tone="accent" />
          )}
        </span>
        <span className={styles.summary}>{summaryText}</span>
      </div>

      {shouldRender && (
        <div
          className={classNames(
            "rf-expand-grid",
            isAnimatingOpen && "is-open",
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <div
              ref={contentRef}
              className={styles.content}
              onScroll={handleScroll}
            >
              <div className={styles.markdown}>
                <Markdown
                  canHaveInteractiveElements={true}
                  onCopyClick={onCopyClick}
                  isStreaming={isStreaming}
                >
                  {deferredContent ?? formattedContent}
                </Markdown>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
