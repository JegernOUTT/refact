import { Search } from "lucide-react";
import React, { useMemo } from "react";
import { Box } from "@radix-ui/themes";
import type { ToolCall } from "../../../services/refact/types";
import { useAppSelector } from "../../../hooks";
import { useThreadId } from "../../../features/Chat/Thread";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { Icon } from "../../ui";
import { useStoredOpen } from "../useStoredOpen";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { ReviewReportView } from "./ReviewReportView";
import { extractReviewReport } from "./reviewReportJson";
import styles from "./GenericTool.module.css";

interface CodeReviewToolProps {
  toolCall: ToolCall;
}

function looksLikeMarkdown(text: string): boolean {
  if (text.includes("```")) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(text)) return true;
  if (/^#{1,6}\s+\S/m.test(text)) return true;
  if (/^\s*([-*+])\s+\S/m.test(text)) return true;
  if (/^\s*\d+\.\s+\S/m.test(text)) return true;
  const hasTableHeader = /^\s*\|.+\|\s*$/m.test(text);
  const hasTableSeparator = /^\s*\|[\s:|-]+\|\s*$/m.test(text);
  return hasTableHeader && hasTableSeparator;
}

export const CodeReviewTool: React.FC<CodeReviewToolProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const threadId = useThreadId();
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    return maybeResult.tool_failed ? "error" : "success";
  }, [maybeResult]);
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const report = useMemo(
    () => (content ? extractReviewReport(content) : null),
    [content],
  );
  const summary = report
    ? `Review: ${report.findings.length} findings${
        report.pipeline?.depth ? ` (${report.pipeline.depth})` : ""
      }`
    : "Review code";
  const shouldRenderMarkdown =
    content !== null && content.length <= 50_000 && looksLikeMarkdown(content);

  return (
    <ToolCard
      icon={
        <Icon
          icon={Search}
          size="md"
          tone={status === "error" ? "danger" : "accent"}
        />
      }
      isOpen={isOpen}
      onToggle={handleToggle}
      status={status}
      summary={summary}
      toolCall={toolCall}
    >
      {report ? (
        <ReviewReportView report={report} />
      ) : content ? (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Result</Box>
          <Box className={styles.resultContent}>
            {shouldRenderMarkdown ? (
              <Box className={styles.markdownContent}>
                <Markdown>{content}</Markdown>
              </Box>
            ) : (
              <ShikiCodeBlock showLineNumbers={false}>
                {content}
              </ShikiCodeBlock>
            )}
          </Box>
        </Box>
      ) : null}
    </ToolCard>
  );
};

export default CodeReviewTool;
