import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Flex, Text, Box, Spinner } from "@radix-ui/themes";
import {
  CopyIcon,
  CheckIcon,
  FileTextIcon,
  ReaderIcon,
} from "@radix-ui/react-icons";
import classNames from "classnames";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../../hooks";
import { isIdeHost } from "../../../utils/isIdeHost";
import { basename } from "./utils";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";
import styles from "./ReportToolCard.module.css";

const MAX_MD_RENDER_CHARS = 50_000;

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

export type ReportVariant = "taskDone" | "report";
type ToolStatus = "running" | "success" | "error";

export interface ReportData {
  summary?: string;
  markdown: string;
  filesChanged?: string[];
  knowledgePath?: string;
}

interface ReportToolCardProps {
  toolCall: ToolCall;
  icon: React.ReactNode;
  defaultSummary: React.ReactNode;
  variant?: ReportVariant;
  meta?: string | null;
  extractReport?: (content: string) => ReportData | null;
  defaultOpen?: boolean;
}

export const ReportToolCard: React.FC<ReportToolCardProps> = ({
  toolCall,
  icon,
  defaultSummary,
  variant = "report",
  meta,
  extractReport,
  defaultOpen = true,
}) => {
  const copyToClipboard = useCopyToClipboard();
  const { newFile, queryPathThenOpenFile } = useEventsBusForIDE();
  const [copied, setCopied] = useState(false);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (maybeResult.tool_failed) return "error";
    return "success";
  }, [maybeResult]);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const reportData = useMemo((): ReportData | null => {
    if (!content) return null;
    if (extractReport) {
      const parsed = extractReport(content);
      if (parsed) return parsed;
    }
    return { markdown: content };
  }, [content, extractReport]);

  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, defaultOpen);
  const [animateContent, setAnimateContent] = useState(false);
  const [bodyReady, setBodyReady] = useState(variant !== "taskDone");

  const handleAnimatedToggle = useCallback(() => {
    setAnimateContent(true);
    handleToggle();
  }, [handleToggle]);

  const summary = reportData?.summary
    ? variant === "taskDone"
      ? reportData.summary
      : reportData.summary
    : defaultSummary;

  const handleCopy = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (reportData?.markdown) {
        copyToClipboard(reportData.markdown);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }
    },
    [reportData?.markdown, copyToClipboard],
  );

  const handleSave = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (reportData?.markdown) {
        newFile(reportData.markdown);
      }
    },
    [reportData?.markdown, newFile],
  );

  const handleFileClick = useCallback(
    (e: React.MouseEvent, filePath: string) => {
      e.stopPropagation();
      void queryPathThenOpenFile({ file_path: filePath });
    },
    [queryPathThenOpenFile],
  );

  const entertainmentText = useMemo(() => {
    if (status !== "running") return null;
    const log = toolCall.subchat_log;
    if (!log || log.length === 0) return null;
    return log.join("\n\n");
  }, [status, toolCall.subchat_log]);
  const deferredEntertainmentText = useStreamingMarkdown(
    entertainmentText,
    status === "running",
  );
  const deferredReportMarkdown = useStreamingMarkdown(
    reportData?.markdown ?? null,
    status === "running",
  );

  useEffect(() => {
    if (bodyReady) return;
    if (!reportData?.markdown) return;
    let cancelled = false;

    const arm = () => {
      if (!cancelled) {
        setBodyReady(true);
      }
    };

    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    let frameId: number | null = null;
    if (typeof globalThis.requestAnimationFrame === "function") {
      frameId = globalThis.requestAnimationFrame(arm);
    } else {
      timeoutId = setTimeout(arm, 16);
    }

    return () => {
      cancelled = true;
      if (
        frameId != null &&
        typeof globalThis.cancelAnimationFrame === "function"
      ) {
        globalThis.cancelAnimationFrame(frameId);
      }
      if (timeoutId != null) {
        clearTimeout(timeoutId);
      }
    };
  }, [bodyReady, reportData?.markdown]);

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
      if (deferredEntertainmentText) {
        addBuddyCrashBreadcrumb("report_progress", deferredEntertainmentText);
      }
      return;
    }

    if (variant === "taskDone") {
      setBuddyCrashHotSlot(
        "report",
        deferredReportMarkdown ??
          reportData?.summary ??
          defaultSummary?.toString() ??
          null,
      );
      if (deferredReportMarkdown) {
        addBuddyCrashBreadcrumb("task_done", deferredReportMarkdown);
      }
      return;
    }

    setBuddyCrashHotSlot("report", null);
  }, [
    defaultSummary,
    deferredEntertainmentText,
    deferredReportMarkdown,
    reportData?.summary,
    status,
    variant,
  ]);

  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen && !!deferredReportMarkdown && bodyReady,
    200,
    animateContent,
  );

  const showActions = status === "success" && !!deferredReportMarkdown;
  const showSaveButton = isIdeHost();

  const header = (
    <Flex
      className={classNames(styles.header)}
      align="center"
      gap="2"
      onClick={handleAnimatedToggle}
    >
      <span className={styles.icon}>
        {status === "running" ? <Spinner size="1" /> : icon}
      </span>
      <Text
        size="1"
        className={classNames(
          styles.summary,
          status === "running" && styles.running,
          status === "error" && styles.error,
          variant === "taskDone" &&
            status === "success" &&
            styles.summaryTaskDone,
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
      {showActions && (
        <span className={styles.actions}>
          <button
            className={classNames(
              styles.actionButton,
              copied && styles.copiedButton,
            )}
            onClick={handleCopy}
            title="Copy report"
          >
            {copied ? <CheckIcon /> : <CopyIcon />}
          </button>
          {showSaveButton && (
            <button
              className={styles.actionButton}
              onClick={handleSave}
              title="Save as file"
            >
              <FileTextIcon />
            </button>
          )}
        </span>
      )}
    </Flex>
  );

  return (
    <div
      className={classNames(
        styles.card,
        variant === "taskDone" ? styles.variantTaskDone : styles.variantReport,
      )}
    >
      <ToolCallTooltip toolCall={toolCall}>{header}</ToolCallTooltip>

      {deferredEntertainmentText && (
        <div
          className={styles.entertainmentContent}
          ref={entertainmentRef}
          onScroll={handleEntertainmentScroll}
        >
          <div className={styles.entertainmentMarkdown}>
            <Markdown canHaveInteractiveElements={false} isStreaming={true}>
              {deferredEntertainmentText}
            </Markdown>
          </div>
        </div>
      )}

      {shouldRender && reportData && deferredReportMarkdown && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
            !animateContent && styles.noTransition,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              {deferredReportMarkdown.length <= MAX_MD_RENDER_CHARS &&
              looksLikeMarkdown(deferredReportMarkdown) ? (
                <Text size="2">
                  <Markdown>{deferredReportMarkdown}</Markdown>
                </Text>
              ) : (
                <ShikiCodeBlock showLineNumbers={false}>
                  {deferredReportMarkdown}
                </ShikiCodeBlock>
              )}
            </Box>

            {reportData.filesChanged && reportData.filesChanged.length > 0 && (
              <Flex gap="2" wrap="wrap" py="1" px="1" align="center">
                <Text size="1" color="gray">
                  Files:
                </Text>
                {reportData.filesChanged.map((f) => (
                  <Text
                    key={f}
                    size="1"
                    className={styles.fileLink}
                    onClick={(e) => handleFileClick(e, f)}
                  >
                    {basename(f)}
                  </Text>
                ))}
              </Flex>
            )}

            {reportData.knowledgePath && (
              <Text
                size="1"
                color="gray"
                as="p"
                style={{ padding: "0 var(--space-1)" }}
              >
                <Flex as="span" align="center" gap="1">
                  <ReaderIcon />
                  Saved to knowledge
                </Flex>
              </Text>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default ReportToolCard;
