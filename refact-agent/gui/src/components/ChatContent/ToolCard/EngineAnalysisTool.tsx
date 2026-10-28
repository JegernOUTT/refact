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
  Palette,
  Radar,
  ScanSearch,
  ShieldAlert,
  type LucideIcon,
} from "lucide-react";
import React, { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptStatus,
} from "../TranscriptItem";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { AnalysisReportView } from "./AnalysisReport";
import { ArtifactsPanel } from "./ArtifactsPanel";
import {
  buildAnalysisReport,
  parseEngineAnalysisJson,
} from "./engineAnalysisJson";
import {
  humanizeToolName,
  parseToolArgs,
  primaryArgument,
} from "./genericToolTitle";
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
  design_system: Palette,
  git_risk: GitBranch,
  image_region: Image,
  mark_elements: MousePointer2,
  pr_blast: Radar,
  security_scan: ShieldAlert,
  ui_probe: ListTree,
  visual_diff: Copy,
};

const ENGINE_ANALYSIS_NAMES: Partial<Record<string, string>> = {
  code_duplication: "Code duplication",
  code_health: "Code health",
  code_map: "Code map",
  code_why: "Code why",
  codegraph_overview: "Codegraph overview",
  contrast_audit: "Contrast audit",
  dead_code: "Dead code",
  design_system: "Design system",
  git_risk: "Git risk",
  image_region: "Image region",
  mark_elements: "Mark elements",
  pr_blast: "PR blast radius",
  security_scan: "Security scan",
  ui_probe: "UI probe",
  visual_diff: "Visual diff",
};

const SCOPE_KEYS = [
  "file_path",
  "path",
  "query",
  "image_path",
  "baseline",
  "task",
] as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function scopeOf(args: Record<string, unknown>): string | undefined {
  for (const key of SCOPE_KEYS) {
    const value = args[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  const changed = args.changed_files;
  if (Array.isArray(changed) && changed.length > 0) {
    const first = typeof changed[0] === "string" ? changed[0] : "";
    return changed.length > 1 ? `${first} +${changed.length - 1}` : first;
  }
  return primaryArgument(args);
}

function count(value: unknown, noun: string): string | undefined {
  if (typeof value !== "number") return undefined;
  return `${value} ${noun}${value === 1 ? "" : "s"}`;
}

function resultMeta(toolName: string, value: unknown): string | undefined {
  if (!isRecord(value)) return undefined;
  if (toolName === "security_scan")
    return count(value.finding_count, "finding");
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
  if (toolName === "code_health" && isRecord(value.aggregate)) {
    const grade = value.aggregate.grade;
    return typeof grade === "string" ? `grade ${grade}` : undefined;
  }
  if (toolName === "code_duplication" && isRecord(value.aggregate)) {
    const pct = value.aggregate.duplication_pct;
    return typeof pct === "number"
      ? `${pct.toFixed(1)}% duplicated`
      : undefined;
  }
  if (toolName === "git_risk" && Array.isArray(value.hotspots))
    return count(value.hotspots.length, "hotspot");
  if (toolName === "code_why" && Array.isArray(value.decisions))
    return count(value.decisions.length, "decision");
  if (toolName === "code_map") return count(value.files_count, "file");
  if (toolName === "codegraph_overview" && isRecord(value.counts))
    return count(value.counts.nodes, "node");
  if (Array.isArray(value.findings))
    return count(value.findings.length, "finding");
  return undefined;
}

function codeMapMarkdown(toolName: string, value: unknown): string | null {
  if (toolName !== "code_map" || !isRecord(value)) return null;
  return typeof value.markdown === "string" ? value.markdown : null;
}

export const EngineAnalysisTool: React.FC<EngineAnalysisToolProps> = ({
  toolCall,
}) => {
  const threadId = useThreadId();
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const unrecorded = useUnrecordedResultState(toolCall.id);
  const failed = Boolean(maybeResult?.tool_failed);
  const status: TranscriptStatus = !maybeResult
    ? unrecorded.status
    : failed
      ? "failed"
      : "done";
  const tone = !maybeResult
    ? unrecorded.status === "running"
      ? "running"
      : "idle"
    : failed
      ? "error"
      : "success";

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const toolName = toolCall.function.name ?? "tool";
  const rawArguments = toolCall.function.arguments;
  const args = useMemo(() => parseToolArgs(rawArguments), [rawArguments]);
  const parsed = useMemo(
    () => (content ? parseEngineAnalysisJson(content) : null),
    [content],
  );
  const report = useMemo(
    () => (parsed ? buildAnalysisReport(toolName, parsed) : null),
    [parsed, toolName],
  );
  const markdown = codeMapMarkdown(toolName, parsed);
  const artifacts = parsed?.artifact ? [parsed] : [];
  const AnalysisIcon = ENGINE_ANALYSIS_ICONS[toolName] ?? Network;
  const scope = scopeOf(args);
  const showAsError = failed && !report;

  const summary: TranscriptFact[] = report
    ? report.facts.map((fact) => ({ label: fact.key, value: fact.value }))
    : [];
  const body = report ? (
    <>
      <AnalysisReportView report={{ ...report, facts: [] }} />
      {markdown ? (
        <div className={styles.markdownContent}>
          <Markdown>{markdown}</Markdown>
        </div>
      ) : null}
    </>
  ) : content ? (
    <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
  ) : null;

  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        artifacts={
          artifacts.length > 0 ? <ArtifactsPanel artifacts={artifacts} /> : null
        }
        className="rf-enter"
        data-status={tone}
        data-testid="engine-analysis-tool"
        error={showAsError ? content : null}
        icon={<AnalysisIcon />}
        meta={resultMeta(toolName, parsed)}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={
          body && !showAsError ? (
            <TranscriptClamp>{body}</TranscriptClamp>
          ) : null
        }
        raw={[
          { label: "Arguments", value: rawArguments },
          ...(content !== null ? [{ label: "JSON", value: content }] : []),
        ]}
        rowTooltip={scope}
        status={status}
        statusLabel={!maybeResult ? unrecorded.statusLabel : undefined}
        summary={summary}
        target={scope}
        verb={ENGINE_ANALYSIS_NAMES[toolName] ?? humanizeToolName(toolName)}
      />
    </ToolCallTooltip>
  );
};

export default EngineAnalysisTool;
