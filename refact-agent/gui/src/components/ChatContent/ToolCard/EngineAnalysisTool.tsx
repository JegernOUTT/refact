import {
  Activity,
  Copy,
  FileWarning,
  GitBranch,
  HelpCircle,
  Image,
  ListTree,
  Map,
  MousePointer2,
  Network,
  Radar,
  ScanSearch,
  ShieldAlert,
  type LucideIcon,
} from "lucide-react";
import React, { useMemo } from "react";
import { Box } from "@radix-ui/themes";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import {
  selectToolResultByThreadAndId,
  selectIsStreamingById,
  selectIsWaitingById,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { Icon } from "../../ui";
import { formatToolDisplayName } from "../../../utils/toolNameAliases";
import { AnalysisReportView } from "./AnalysisReport";
import { ArtifactsPanel } from "./ArtifactsPanel";
import {
  buildAnalysisReport,
  parseEngineAnalysisJson,
} from "./engineAnalysisJson";
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
  contrast_audit: ScanSearch,
  dead_code: FileWarning,
  git_risk: GitBranch,
  image_region: Image,
  mark_elements: MousePointer2,
  pr_blast: Radar,
  security_scan: ShieldAlert,
  ui_probe: ListTree,
  visual_diff: Copy,
};

function formatArgs(argsStr: string): string {
  try {
    const parsed: unknown = JSON.parse(argsStr);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))
      return argsStr;
    const entries = Object.entries(parsed);
    if (entries.length === 0) return "";
    return entries
      .map(([key, value]) =>
        [key, typeof value === "string" ? value : JSON.stringify(value)].join(
          "=",
        ),
      )
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
  return normalized.length <= maxLength
    ? normalized
    : normalized.slice(0, maxLength - 1).concat("…");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function resultMeta(toolName: string, value: unknown): string | undefined {
  if (!isRecord(value)) return undefined;
  if (toolName === "security_scan" && typeof value.finding_count === "number")
    return `${value.finding_count} findings`;
  if (
    toolName === "pr_blast" &&
    typeof value.impacted_file_count === "number" &&
    typeof value.risk_score === "number"
  )
    return `${
      value.impacted_file_count
    } files · risk ${value.risk_score.toFixed(2)}`;
  if (
    toolName === "dead_code" &&
    typeof value.shown === "number" &&
    typeof value.total_candidates === "number"
  )
    return `${value.shown}/${value.total_candidates} candidates`;
  return undefined;
}

function codeMapMarkdown(toolName: string, value: unknown): string | null {
  if (toolName !== "code_map" || !isRecord(value)) return null;
  return typeof value.markdown === "string" ? value.markdown : null;
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
    )
      return "error";
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
    return argsPreview ? (
      <>
        {displayName} <span className={styles.args}>{argsPreview}</span>
      </>
    ) : (
      displayName
    );
  }, [toolName, argsPreview]);

  const parsed = useMemo(
    () => (content ? parseEngineAnalysisJson(content) : null),
    [content],
  );
  const report = useMemo(
    () => (parsed ? buildAnalysisReport(toolName, parsed) : null),
    [parsed, toolName],
  );
  const markdown = codeMapMarkdown(toolName, parsed);
  const meta = resultMeta(toolName, parsed);
  const artifacts = parsed?.artifact ? [parsed] : [];
  const AnalysisIcon = ENGINE_ANALYSIS_ICONS[toolName] ?? Network;

  return (
    <>
      <span data-testid="engine-analysis-tool" hidden />
      <ToolCard
        icon={
          <Icon
            icon={AnalysisIcon}
            size="md"
            tone={status === "error" ? "danger" : "accent"}
          />
        }
        summary={summary}
        meta={meta}
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
        {(report !== null || content !== null) && (
          <Box className={styles.section}>
            <Box className={styles.sectionLabel}>Result</Box>
            {report ? (
              <AnalysisReportView report={report} />
            ) : content ? (
              <Box className={styles.resultContent}>
                <ShikiCodeBlock showLineNumbers={false}>
                  {content}
                </ShikiCodeBlock>
              </Box>
            ) : null}
            {report && markdown && (
              <Box className={styles.markdownContent}>
                <Markdown>{markdown}</Markdown>
              </Box>
            )}
            {artifacts.length > 0 && <ArtifactsPanel artifacts={artifacts} />}
          </Box>
        )}
      </ToolCard>
    </>
  );
};

export default EngineAnalysisTool;
