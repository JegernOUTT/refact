import React, { useCallback, useEffect, useMemo, useRef } from "react";
import { Text, Box } from "@radix-ui/themes";
import { LoaderCircle } from "lucide-react";
import { Icon } from "../../ui";
import classNames from "classnames";
import { useAutoExpandCollapse, ToolStatus } from "./useAutoExpandCollapse";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../useChatScrollAnchor";
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
  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();

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
  const handleOpenChange = useCallback(() => {
    preserveScrollAnchor(onToggle);
  }, [onToggle, preserveScrollAnchor]);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const shouldRenderMarkdown =
    content &&
    content.length <= MAX_MD_RENDER_CHARS &&
    looksLikeMarkdown(content);

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
  const deferredContent = useStreamingMarkdown(content, status === "running");

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
    <span className={styles.titleRow}>
      <span className={styles.icon}>
        {status === "running" ? (
          <Icon
            className="rf-spin"
            icon={LoaderCircle}
            size="sm"
            tone="accent"
          />
        ) : (
          icon
        )}
      </span>
      <span
        className={classNames(
          styles.summary,
          status === "error" && styles.error,
        )}
      >
        {status === "running" ? (
          <span className="rf-text-shimmer">{summary}</span>
        ) : (
          summary
        )}
      </span>
      {meta && <span className={styles.meta}>{meta}</span>}
      {status === "error" && <span className={styles.errorBadge}>failed</span>}
    </span>
  );

  const hasContentBody = !!deferredContent;
  const card = (
    <AnimatedCollapsible
      animate={animate}
      className={classNames(
        "rf-enter",
        styles.card,
        status === "running" && styles.running,
        !hasContentBody && styles.withoutBody,
      )}
      data-status={status}
      header={header}
      onKeyDownCapture={prepareScrollAnchor}
      onMouseDownCapture={prepareScrollAnchor}
      onOpenChange={handleOpenChange}
      onPointerDownCapture={prepareScrollAnchor}
      open={isOpen}
      status={status}
      variant="compact"
    >
      {hasContentBody ? (
        <Box className={styles.content}>
          {shouldRenderMarkdown ? (
            <Text size="2">
              <Markdown isStreaming={status === "running"}>
                {deferredContent}
              </Markdown>
            </Text>
          ) : (
            <ShikiCodeBlock showLineNumbers={false}>
              {deferredContent}
            </ShikiCodeBlock>
          )}
        </Box>
      ) : null}
    </AnimatedCollapsible>
  );

  return (
    <div className={styles.stack}>
      <ToolCallTooltip toolCall={toolCall}>{card}</ToolCallTooltip>

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
    </div>
  );
};

export default StreamingToolCard;
