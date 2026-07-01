import {
  Activity,
  Copy,
  GitBranch,
  HelpCircle,
  Map,
  Network,
  type LucideIcon,
} from "lucide-react";
import React, { useMemo } from "react";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import {
  selectToolResultByThreadAndId,
  selectIsStreamingById,
  selectIsWaitingById,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Markdown } from "../../Markdown";
import { formatToolDisplayName } from "../../../utils/toolNameAliases";
import styles from "./GenericTool.module.css";

interface EngineAnalysisToolProps {
  toolCall: ToolCall;
}

const ENGINE_ANALYSIS_ICONS: Partial<Record<string, LucideIcon>> = {
  code_duplication: Copy,
  code_health: Activity,
  code_map: Map,
  code_why: HelpCircle,
  codegraph_overview: Network,
  git_risk: GitBranch,
};

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

export const EngineAnalysisTool: React.FC<EngineAnalysisToolProps> = ({
  toolCall,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
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
  const AnalysisIcon = ENGINE_ANALYSIS_ICONS[toolName] ?? Network;

  return (
    <>
      <span data-testid="engine-analysis-tool" hidden />
      <ToolCard
        icon={<AnalysisIcon />}
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

        {content && (
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
        )}
      </ToolCard>
    </>
  );
};

export default EngineAnalysisTool;
