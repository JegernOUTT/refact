import { Settings } from "lucide-react";
import React, { useMemo, useState } from "react";
import { Badge, Box, Button, Flex } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks/useAppSelector";
import {
  selectToolResultByThreadAndId,
  selectIsStreamingById,
  selectIsWaitingById,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import {
  getToolEnrichment,
  type ToolCall,
} from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Markdown } from "../../Markdown";
import { formatToolDisplayName } from "../../../utils/toolNameAliases";
import styles from "./GenericTool.module.css";

interface GenericToolProps {
  toolCall: ToolCall;
}

const DEFAULT_RESULT_HEAD_CHARS = 30_000;
const DEFAULT_RESULT_TAIL_CHARS = 10_000;

function formatArgs(argsStr: string): string {
  try {
    const args = JSON.parse(argsStr) as Record<string, unknown>;
    const entries = Object.entries(args);
    if (entries.length === 0) return "";
    return entries
      .map(([key, value]) => {
        const valueStr =
          typeof value === "string" ? value : JSON.stringify(value);
        return [key, valueStr].join("=");
      })
      .join(", ");
  } catch {
    return argsStr;
  }
}

function formatRawArgs(argsStr: string): string {
  try {
    return JSON.stringify(JSON.parse(argsStr) as unknown, null, 2);
  } catch {
    return argsStr;
  }
}

function truncatePreview(text: string, maxLength = 120): string {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxLength) return normalized;
  return normalized.slice(0, maxLength - 1).concat("…");
}

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

function capToolResult(text: string): {
  rendered: string;
  hiddenChars: number;
  capped: boolean;
} {
  const maxChars = DEFAULT_RESULT_HEAD_CHARS + DEFAULT_RESULT_TAIL_CHARS;
  if (text.length <= maxChars) {
    return { rendered: text, hiddenChars: 0, capped: false };
  }

  const hiddenChars = text.length - maxChars;
  return {
    rendered: `${text.slice(
      0,
      DEFAULT_RESULT_HEAD_CHARS,
    )}\n… output capped in UI (${hiddenChars} chars hidden) …\n${text.slice(
      -DEFAULT_RESULT_TAIL_CHARS,
    )}`,
    hiddenChars,
    capped: true,
  };
}

export const GenericTool: React.FC<GenericToolProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const [showFullOutput, setShowFullOutput] = useState(false);
  const threadId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, threadId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, threadId),
  );

  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult, isStreaming, isWaiting]);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const enrichment = getToolEnrichment(maybeResult?.extra);

  const toolName = toolCall.function.name ?? "tool";
  const argsPreview = truncatePreview(formatArgs(toolCall.function.arguments));
  const rawArgs = useMemo(
    () => formatRawArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );

  const summary = useMemo(() => {
    const displayName = formatToolDisplayName(toolName);
    if (argsPreview) {
      return (
        <>
          {displayName} <span className={styles.args}>{argsPreview}</span>
        </>
      );
    }
    return displayName;
  }, [toolName, argsPreview]);

  const shouldRenderMarkdown =
    content && content.length <= 50000 && looksLikeMarkdown(content);
  const cappedResult = useMemo(
    () => (content ? capToolResult(content) : null),
    [content],
  );
  const renderedContent =
    content && showFullOutput ? content : cappedResult?.rendered ?? content;
  const isCappedByDefault = Boolean(cappedResult?.capped && !showFullOutput);

  return (
    <>
      <span data-testid="generic-tool" hidden />
      <ToolCard
        icon={<Settings />}
        summary={summary}
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Arguments</Box>
          <Box className={styles.resultContent}>
            <ShikiCodeBlock showLineNumbers={false}>{rawArgs}</ShikiCodeBlock>
          </Box>
        </Box>

        {enrichment &&
          !enrichment.privacy?.restricted &&
          enrichment.references.length > 0 && (
            <Box className={styles.section} data-testid="tool-enrichment">
              <Box className={styles.sectionLabel}>References</Box>
              <Flex gap="2" wrap="wrap">
                {enrichment.references.map((reference) => (
                  <Badge
                    key={`${reference.kind}:${reference.target}`}
                    color={reference.status === "failed" ? "red" : "gray"}
                    variant="soft"
                  >
                    {reference.kind}: {reference.label ?? reference.target}
                    {reference.line1
                      ? `:${reference.line1}-${
                          reference.line2 ?? reference.line1
                        }`
                      : ""}
                    {reference.count ? ` (${reference.count})` : ""}
                    {reference.status ? ` (${reference.status})` : ""}
                  </Badge>
                ))}
              </Flex>
            </Box>
          )}

        {content && (
          <Box className={styles.section}>
            <Box className={styles.sectionLabel}>Result</Box>
            <Box className={styles.resultContent}>
              {isCappedByDefault && cappedResult ? (
                <Box mb="2">
                  <Badge variant="soft" color="gray">
                    Showing first {DEFAULT_RESULT_HEAD_CHARS.toLocaleString()}{" "}
                    and last {DEFAULT_RESULT_TAIL_CHARS.toLocaleString()} chars
                    ({cappedResult.hiddenChars.toLocaleString()} hidden)
                  </Badge>
                </Box>
              ) : null}
              {cappedResult?.capped ? (
                <Box mb="2">
                  <Button
                    type="button"
                    size="1"
                    variant="soft"
                    color="gray"
                    onClick={() => setShowFullOutput((value) => !value)}
                  >
                    {showFullOutput ? "Show capped output" : "Show full output"}
                  </Button>
                </Box>
              ) : null}
              {renderedContent && shouldRenderMarkdown && !isCappedByDefault ? (
                <Box className={styles.markdownContent}>
                  <Markdown>{renderedContent}</Markdown>
                </Box>
              ) : (
                <ShikiCodeBlock showLineNumbers={false}>
                  {renderedContent}
                </ShikiCodeBlock>
              )}
            </Box>
          </Box>
        )}
      </ToolCard>
    </>
  );
};

export default GenericTool;
