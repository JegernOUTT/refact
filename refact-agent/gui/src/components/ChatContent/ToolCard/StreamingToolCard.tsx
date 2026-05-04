import React, { useCallback, useEffect, useMemo, useRef } from "react";
import { Flex, Text, Box, Spinner } from "@radix-ui/themes";
import classNames from "classnames";
import { useAutoExpandCollapse, ToolStatus } from "./useAutoExpandCollapse";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";
import styles from "./StreamingToolCard.module.css";

const MAX_MD_RENDER_CHARS = 50_000;
const MAX_STREAMING_PROGRESS_CHARS = 500;

function looksLikeMarkdown(text: string): boolean {
  if (text.includes("```")) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(text)) return true;
  if (/^#{1,6}\s+\S/m.test(text)) return true;
  if (/^\s*([-*+])\s+\S/m.test(text)) return true;
  if (/^\s*\d+\.\s+\S/m.test(text)) return true;
  const hasTableHeader = /^\s*\|.+\|\s*$/m.test(text);
  const hasTableSep = /^\s*\|[\s:|-]+\|\s*$/m.test(text);
  if (hasTableHeader && hasTableSep) return true;
  return false;
}

interface StreamingToolCardProps {
  toolCall: ToolCall;
  icon: React.ReactNode;
  summary: React.ReactNode;
  meta?: string | null;
}

export const StreamingToolCard: React.FC<StreamingToolCardProps> = ({
  toolCall,
  icon,
  summary,
  meta,
}) => {
  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (maybeResult.tool_failed) return "error";
    return "success";
  }, [maybeResult]);

  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const { isOpen, onToggle, animate } = useAutoExpandCollapse({
    status,
    storeKey,
  });

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const shouldRenderMarkdown =
    content &&
    content.length <= MAX_MD_RENDER_CHARS &&
    looksLikeMarkdown(content);

  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen && !!content,
    200,
    animate,
  );

  const entertainmentMessage = useMemo(() => {
    if (status !== "running") return null;
    const log = toolCall.subchat_log;
    if (!log || log.length === 0) return null;
    const last = log[log.length - 1];
    const stepMatch = last.match(/^(\d+\/\d+):\s*([\s\S]+)$/);
    if (stepMatch) {
      return {
        step: stepMatch[1],
        text: stepMatch[2].trim().slice(-MAX_STREAMING_PROGRESS_CHARS),
      };
    }
    return { step: null, text: last.slice(-MAX_STREAMING_PROGRESS_CHARS) };
  }, [status, toolCall.subchat_log]);

  const entertainmentText = entertainmentMessage?.step
    ? `${entertainmentMessage.step}: ${entertainmentMessage.text}`
    : entertainmentMessage?.text ?? null;
  const deferredEntertainmentText = useStreamingMarkdown(
    entertainmentText,
    status === "running",
  );
  const deferredContent = useStreamingMarkdown(
    isOpen ? content : null,
    status === "running",
  );

  const entertainmentRef = useRef<HTMLDivElement | null>(null);
  const userScrolledRef = useRef(false);

  const handleEntertainmentScroll = useCallback(() => {
    const el = entertainmentRef.current;
    if (!el) return;
    const isAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 20;
    userScrolledRef.current = !isAtBottom;
  }, []);

  useEffect(() => {
    if (status !== "running") return;
    const el = entertainmentRef.current;
    if (!el) return;
    if (userScrolledRef.current) return;
    if (el.scrollTop + el.clientHeight + 20 < el.scrollHeight) {
      el.scrollTop = el.scrollHeight;
    }
  }, [status, deferredEntertainmentText]);

  useEffect(() => {
    if (status === "running") {
      setBuddyCrashHotSlot(
        "tool",
        deferredEntertainmentText ?? summary?.toString() ?? null,
      );
      if (deferredEntertainmentText) {
        addBuddyCrashBreadcrumb("tool_progress", deferredEntertainmentText);
      }
      return;
    }

    setBuddyCrashHotSlot("tool", null);
  }, [deferredEntertainmentText, status, summary]);

  const header = (
    <Flex
      className={classNames(styles.header, status === "error" && styles.error)}
      align="center"
      gap="2"
      onClick={onToggle}
    >
      <span className={styles.icon}>
        {status === "running" ? <Spinner size="1" /> : icon}
      </span>
      <Text
        size="1"
        className={classNames(
          styles.summary,
          status === "running" && styles.running,
        )}
      >
        {summary}
      </Text>
      {meta && (
        <Text size="1" color="gray" className={styles.meta}>
          {meta}
        </Text>
      )}
      {status === "error" && (
        <Text size="1" color="red" className={styles.errorBadge}>
          failed
        </Text>
      )}
    </Flex>
  );

  return (
    <div className={styles.card}>
      <ToolCallTooltip toolCall={toolCall}>{header}</ToolCallTooltip>

      {deferredEntertainmentText && (
        <div
          className={styles.entertainmentContent}
          ref={entertainmentRef}
          onScroll={handleEntertainmentScroll}
        >
          <Text size="1" color="gray" className={styles.entertainmentText}>
            {deferredEntertainmentText}
          </Text>
        </div>
      )}

      {shouldRender && deferredContent && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
            !animate && styles.noTransition,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              {shouldRenderMarkdown ? (
                <Text size="2">
                  <Markdown>{deferredContent}</Markdown>
                </Text>
              ) : (
                <ShikiCodeBlock showLineNumbers={false}>
                  {deferredContent}
                </ShikiCodeBlock>
              )}
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

export default StreamingToolCard;
