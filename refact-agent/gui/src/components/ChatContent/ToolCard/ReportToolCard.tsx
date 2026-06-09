import { Copy, Check, FileText, BookOpen, LoaderCircle } from "lucide-react";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Flex, Text, Box } from "@radix-ui/themes";
import { Icon } from "../../ui";
import classNames from "classnames";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../useChatScrollAnchor";
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

export type ReportVariant = "taskDone" | "plan" | "report";
type ToolStatus = "running" | "success" | "error";

export interface ReportData {
  summary?: string;
  markdown: string;
  filesChanged?: string[];
  knowledgePath?: string;
}

const REPORT_VARIANT_CLASSES = {
  taskDone: styles.variantTaskDone,
  plan: styles.variantPlan,
  report: styles.variantReport,
} satisfies Record<ReportVariant, string>;

interface ReportToolCardProps {
  toolCall: ToolCall;
  icon: React.ReactNode;
  defaultSummary: React.ReactNode;
  variant?: ReportVariant;
  meta?: string | null;
  extractReport?: (content: string) => ReportData | null;
  defaultOpen?: boolean;
  unboundedContent?: boolean;
}

export const ReportToolCard: React.FC<ReportToolCardProps> = ({
  toolCall,
  icon,
  defaultSummary,
  variant = "report",
  meta,
  extractReport,
  defaultOpen = true,
  unboundedContent = false,
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
  const [isOpen, , setOpen] = useStoredOpen(storeKey, defaultOpen);
  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();
  const [animateContent, setAnimateContent] = useState(false);
  const [bodyReady, setBodyReady] = useState(variant !== "taskDone");

  const handleOpenChange = useCallback(
    (open: boolean) => {
      setAnimateContent(true);
      preserveScrollAnchor(() => setOpen(open));
    },
    [preserveScrollAnchor, setOpen],
  );

  const summary = reportData?.summary ?? defaultSummary;

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
    return log[log.length - 1].slice(-MAX_STREAMING_PROGRESS_CHARS);
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
    if (variant !== "taskDone") {
      setBodyReady(true);
      return;
    }
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
  }, [bodyReady, reportData?.markdown, variant]);

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

  const showActions =
    status === "success" && isOpen && !!deferredReportMarkdown;
  const showSaveButton = isIdeHost();
  const variantClass = REPORT_VARIANT_CLASSES[variant];
  const hasReportBody = !!deferredReportMarkdown && bodyReady;

  const actions = showActions ? (
    <>
      <button
        className={classNames(
          styles.actionButton,
          copied && styles.copiedButton,
        )}
        onClick={handleCopy}
        title="Copy report"
        type="button"
      >
        {copied ? <Check /> : <Copy />}
      </button>
      {showSaveButton && (
        <button
          className={styles.actionButton}
          onClick={handleSave}
          title="Save as file"
          type="button"
        >
          <FileText />
        </button>
      )}
    </>
  ) : null;

  const header = (
    <span
      className={classNames(
        styles.titleRow,
        status === "running" && "rf-active-pulse",
      )}
    >
      <span className={styles.icon}>
        {status === "running" ? (
          <Icon icon={LoaderCircle} size="sm" tone="accent" />
        ) : (
          icon
        )}
      </span>
      <span
        className={classNames(
          styles.summary,
          status === "error" && styles.error,
          variant === "taskDone" &&
            status === "success" &&
            styles.summaryTaskDone,
        )}
      >
        {summary}
      </span>
      {meta && <span className={styles.meta}>{meta}</span>}
      {status === "error" && <span className={styles.errorBadge}>failed</span>}
    </span>
  );

  const card = (
    <AnimatedCollapsible
      animate={animateContent}
      actions={actions}
      className={classNames(
        "rf-enter",
        styles.card,
        variantClass,
        status === "running" && styles.running,
        !hasReportBody && styles.withoutBody,
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
      {hasReportBody && reportData ? (
        <>
          <Box
            className={classNames(
              styles.content,
              unboundedContent && styles.contentUnbounded,
            )}
          >
            {deferredReportMarkdown.length <= MAX_MD_RENDER_CHARS &&
            looksLikeMarkdown(deferredReportMarkdown) ? (
              <Text size="2">
                <Markdown isStreaming={status === "running"}>
                  {deferredReportMarkdown}
                </Markdown>
              </Text>
            ) : (
              <ShikiCodeBlock showLineNumbers={false}>
                {deferredReportMarkdown}
              </ShikiCodeBlock>
            )}
          </Box>

          {reportData.filesChanged && reportData.filesChanged.length > 0 && (
            <Flex
              className={styles.fileFooter}
              gap="2"
              wrap="wrap"
              align="center"
            >
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
            <Text size="1" color="gray" as="p" className={styles.knowledgePath}>
              <Flex as="span" align="center" gap="1">
                <BookOpen />
                Saved to knowledge
              </Flex>
            </Text>
          )}
        </>
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

export default ReportToolCard;
