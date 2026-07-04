import {
  Activity,
  Copy,
  FileWarning,
  GitBranch,
  HelpCircle,
  Map,
  Network,
  Radar,
  ShieldAlert,
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
import { Badge, Chip, Icon } from "../../ui";
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
  dead_code: FileWarning,
  git_risk: GitBranch,
  pr_blast: Radar,
  security_scan: ShieldAlert,
};

type Severity = "Low" | "Medium" | "High" | "Critical";

type SecurityFindingPreview = {
  severity: Severity;
  line: number;
  rule: string;
  snippet: string;
};

type SecurityScanPreview = {
  path: string;
  lang: string;
  findings: number;
  counts: Record<Severity, number>;
  samples: SecurityFindingPreview[];
};

type BlastImpactPreview = {
  distance: number;
  symbol: string;
  path: string;
  via: string;
  kind: "behavioral" | "structural";
};

type BlastReviewerPreview = {
  author: string;
  score: number;
};

type PrBlastPreview = {
  maxDepth: number;
  changedCount: number;
  changedFiles: string[];
  directCount: number;
  transitiveCount: number;
  impactedFiles: number;
  riskScore: number;
  partial: boolean;
  warning: string | null;
  impacts: BlastImpactPreview[];
  reviewers: BlastReviewerPreview[];
};

type DeadCodePreviewEntry = {
  confidence: number;
  line: number;
  name: string;
  path: string;
  detail: string;
};

type DeadCodePreview = {
  shown: number;
  total: number;
  partial: boolean;
  warning: string | null;
  entries: DeadCodePreviewEntry[];
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

function toInt(value: string | undefined): number {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function toFloat(value: string | undefined): number {
  const parsed = Number.parseFloat(value ?? "");
  return Number.isFinite(parsed) ? parsed : 0;
}

function formatNumber(value: number): string {
  return Number.isFinite(value) ? value.toLocaleString() : "—";
}

function formatScore(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (Math.abs(value) >= 1) return value.toFixed(2);
  return value.toFixed(4);
}

function severityTone(
  severity: Severity,
): React.ComponentProps<typeof Badge>["tone"] {
  if (severity === "Critical" || severity === "High") return "danger";
  if (severity === "Medium") return "warning";
  return "success";
}

function impactTone(
  kind: BlastImpactPreview["kind"],
): React.ComponentProps<typeof Badge>["tone"] {
  return kind === "structural" ? "warning" : "accent";
}

function parseSecurityScan(content: string): SecurityScanPreview | null {
  const found = content.match(
    /Security scan for `([^`]+)` found (\d+) findings \(lang: ([^)]+)\)\./,
  );
  const noFindings = content.match(
    /Security scan for `([^`]+)` found no findings \(lang: ([^)]+)\)\./,
  );
  if (!found && !noFindings) return null;

  const counts: Record<Severity, number> = {
    Low: 0,
    Medium: 0,
    High: 0,
    Critical: 0,
  };
  const countMatch = content.match(
    /Severity counts: Critical=(\d+) High=(\d+) Medium=(\d+) Low=(\d+)/,
  );
  if (countMatch) {
    counts.Critical = toInt(countMatch[1]);
    counts.High = toInt(countMatch[2]);
    counts.Medium = toInt(countMatch[3]);
    counts.Low = toInt(countMatch[4]);
  }

  const samples: SecurityFindingPreview[] = [];
  const findingRe =
    /^\s+.+?:(\d+) \[(Critical|High|Medium|Low)\] (.+?) — (.+)$/gm;
  for (const match of content.matchAll(findingRe)) {
    samples.push({
      line: toInt(match[1]),
      severity: match[2] as Severity,
      rule: match[3].trim(),
      snippet: match[4].trim(),
    });
  }

  if (found) {
    return {
      path: found[1],
      findings: toInt(found[2]),
      lang: found[3],
      counts,
      samples,
    };
  }

  if (!noFindings) return null;

  return {
    path: noFindings[1],
    findings: 0,
    lang: noFindings[2],
    counts,
    samples,
  };
}

function parsePrBlast(content: string): PrBlastPreview | null {
  const header = content.match(
    /PR blast radius \(max depth (\d+)\) for (\d+) changed files:/,
  );
  if (!header) return null;

  const changedFiles = Array.from(
    content.matchAll(/^\s+changed: (.+)$/gm),
    (match) => match[1].trim(),
  ).filter(Boolean);
  const directCount = toInt(
    content.match(/Directly impacted symbols \((\d+)\):/)?.[1],
  );
  const transitiveCount = toInt(
    content.match(/Transitively impacted symbols \((\d+)\):/)?.[1],
  );
  const impactedFiles = toInt(content.match(/Impacted files: (\d+)/)?.[1]);
  const riskScore = toFloat(content.match(/Risk score: ([\d.]+)/)?.[1]);
  const indexLine = content.match(/Index state: .+ partial=(true|false)/);
  const warning = content.startsWith("⚠") ? content.split("\n")[0] : null;
  const impacts = Array.from(
    content.matchAll(
      /^\s+d(\d+) (.+?) @ (.+?) via (.+?) \((behavioral|structural)\)$/gm,
    ),
    (match): BlastImpactPreview => ({
      distance: toInt(match[1]),
      symbol: match[2].trim(),
      path: match[3].trim(),
      via: match[4].trim(),
      kind: match[5] as BlastImpactPreview["kind"],
    }),
  );
  const reviewers = Array.from(
    content.matchAll(/^\s+(.+?) \(score ([\d.]+)\)$/gm),
    (match): BlastReviewerPreview => ({
      author: match[1].trim(),
      score: toFloat(match[2]),
    }),
  ).filter((reviewer) => reviewer.author.length > 0);

  return {
    maxDepth: toInt(header[1]),
    changedCount: toInt(header[2]),
    changedFiles,
    directCount,
    transitiveCount,
    impactedFiles,
    riskScore,
    partial: indexLine?.[1] === "true" || warning !== null,
    warning,
    impacts,
    reviewers,
  };
}

function parseDeadCode(content: string): DeadCodePreview | null {
  const header = content.match(
    /Dead code candidates: (\d+) shown of (\d+) matching candidates\./,
  );
  if (!header) return null;

  const warning = content.startsWith("⚠") ? content.split("\n")[0] : null;
  const partial =
    content.match(/Index state: .+ partial=(true|false)/)?.[1] === "true" ||
    warning !== null;
  const entries: DeadCodePreviewEntry[] = [];
  let currentPath = "";
  for (const line of content.split("\n")) {
    const pathMatch = line.match(/^([^\s].+):$/);
    if (pathMatch && !line.startsWith("Index state")) {
      currentPath = pathMatch[1];
      continue;
    }
    const entryMatch = line.match(
      /^\s+([\d.]+)\s+line\s+(\d+)\s+(.+?)\s+—\s+(.+)$/,
    );
    if (entryMatch) {
      entries.push({
        confidence: toFloat(entryMatch[1]),
        line: toInt(entryMatch[2]),
        name: entryMatch[3].trim(),
        path: currentPath,
        detail: entryMatch[4].trim(),
      });
    }
  }

  return {
    shown: toInt(header[1]),
    total: toInt(header[2]),
    partial,
    warning,
    entries,
  };
}

function MetricCard({
  label,
  value,
}: {
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div className={styles.metricCard}>
      <span className={styles.metricLabel}>{label}</span>
      <span className={styles.metricValue}>{value}</span>
    </div>
  );
}

function PreviewSection({
  children,
  title,
}: {
  children: React.ReactNode;
  title: string;
}) {
  return (
    <Box className={styles.section}>
      <Box className={styles.sectionLabel}>{title}</Box>
      <Box className={styles.summaryGrid}>{children}</Box>
    </Box>
  );
}

function SecurityScanResult({ preview }: { preview: SecurityScanPreview }) {
  const severities: Severity[] = ["Critical", "High", "Medium", "Low"];

  return (
    <PreviewSection title="Security findings">
      <div className={styles.metricGrid}>
        <MetricCard label="Findings" value={formatNumber(preview.findings)} />
        <MetricCard label="Language" value={preview.lang || "—"} />
      </div>
      <div className={styles.chipRow}>
        <Chip radius="chip">{preview.path}</Chip>
        {severities.map((severity) => (
          <Badge key={severity} tone={severityTone(severity)} variant="soft">
            {severity} {formatNumber(preview.counts[severity])}
          </Badge>
        ))}
      </div>
      {preview.samples.length > 0 ? (
        <ul className={styles.itemList}>
          {preview.samples.slice(0, 6).map((finding, index) => (
            <li className={styles.itemCard} key={`${finding.rule}-${index}`}>
              <div className={styles.badgeRow}>
                <Badge tone={severityTone(finding.severity)} variant="soft">
                  {finding.severity}
                </Badge>
                <Badge tone="muted" variant="outline">
                  {finding.rule}
                </Badge>
                <Badge tone="muted" variant="outline">
                  Line {formatNumber(finding.line)}
                </Badge>
              </div>
              <code className={styles.itemSnippet}>{finding.snippet}</code>
            </li>
          ))}
        </ul>
      ) : (
        <p className={styles.emptyText}>No security findings in the result.</p>
      )}
    </PreviewSection>
  );
}

function PrBlastResult({ preview }: { preview: PrBlastPreview }) {
  const structuralCount = preview.impacts.filter(
    (impact) => impact.kind === "structural",
  ).length;
  const behavioralCount = preview.impacts.length - structuralCount;

  return (
    <PreviewSection title="Blast-radius summary">
      <div className={styles.metricGrid}>
        <MetricCard
          label="Changed"
          value={formatNumber(preview.changedCount)}
        />
        <MetricCard label="Direct" value={formatNumber(preview.directCount)} />
        <MetricCard
          label="Transitive"
          value={formatNumber(preview.transitiveCount)}
        />
        <MetricCard
          label="Impacted files"
          value={formatNumber(preview.impactedFiles)}
        />
        <MetricCard label="Risk" value={formatScore(preview.riskScore)} />
      </div>
      <div className={styles.chipRow}>
        <Badge tone="muted" variant="outline">
          depth {formatNumber(preview.maxDepth)}
        </Badge>
        {preview.partial ? (
          <Badge tone="warning" variant="soft">
            partial index
          </Badge>
        ) : null}
        <Badge tone="accent" variant="soft">
          behavioral {formatNumber(behavioralCount)}
        </Badge>
        <Badge tone="warning" variant="soft">
          structural {formatNumber(structuralCount)}
        </Badge>
      </div>
      {preview.warning ? (
        <p className={styles.emptyText}>{preview.warning}</p>
      ) : null}
      {preview.changedFiles.length > 0 ? (
        <div className={styles.chipRow}>
          {preview.changedFiles.slice(0, 8).map((file) => (
            <Chip key={file} radius="chip">
              {file}
            </Chip>
          ))}
        </div>
      ) : null}
      {preview.reviewers.length > 0 ? (
        <div className={styles.chipRow}>
          {preview.reviewers.slice(0, 5).map((reviewer) => (
            <Chip key={reviewer.author} radius="chip">
              {reviewer.author} · {formatScore(reviewer.score)}
            </Chip>
          ))}
        </div>
      ) : null}
      {preview.impacts.length > 0 ? (
        <ul className={styles.itemList}>
          {preview.impacts.slice(0, 8).map((impact, index) => (
            <li
              className={styles.itemCard}
              key={`${impact.path}-${impact.symbol}-${index}`}
            >
              <span className={styles.itemTitle}>{impact.symbol}</span>
              <span className={styles.itemMeta}>{impact.path}</span>
              <div className={styles.badgeRow}>
                <Badge tone="muted" variant="outline">
                  d{formatNumber(impact.distance)}
                </Badge>
                <Badge tone={impactTone(impact.kind)} variant="soft">
                  {impact.kind}
                </Badge>
                <Badge tone="muted" variant="outline">
                  via {impact.via}
                </Badge>
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className={styles.emptyText}>No reverse dependencies found.</p>
      )}
    </PreviewSection>
  );
}

function DeadCodeResult({ preview }: { preview: DeadCodePreview }) {
  return (
    <PreviewSection title="Dead-code candidates">
      <div className={styles.metricGrid}>
        <MetricCard label="Shown" value={formatNumber(preview.shown)} />
        <MetricCard label="Matching" value={formatNumber(preview.total)} />
      </div>
      {preview.partial ? (
        <div className={styles.chipRow}>
          <Badge tone="warning" variant="soft">
            partial index
          </Badge>
        </div>
      ) : null}
      {preview.warning ? (
        <p className={styles.emptyText}>{preview.warning}</p>
      ) : null}
      {preview.entries.length > 0 ? (
        <ul className={styles.itemList}>
          {preview.entries.slice(0, 8).map((entry, index) => (
            <li
              className={styles.itemCard}
              key={`${entry.path}-${entry.name}-${index}`}
            >
              <span className={styles.itemTitle}>{entry.name}</span>
              <span className={styles.itemMeta}>{entry.path}</span>
              <div className={styles.badgeRow}>
                <Badge
                  tone={entry.confidence >= 0.8 ? "danger" : "warning"}
                  variant="soft"
                >
                  confidence {formatScore(entry.confidence)}
                </Badge>
                <Badge tone="muted" variant="outline">
                  line {formatNumber(entry.line)}
                </Badge>
              </div>
              <span className={styles.itemMeta}>{entry.detail}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className={styles.emptyText}>No dead-code candidates matched.</p>
      )}
    </PreviewSection>
  );
}

function SpecializedResult({
  content,
  toolName,
}: {
  content: string;
  toolName: string;
}) {
  if (toolName === "security_scan") {
    const preview = parseSecurityScan(content);
    return preview ? <SecurityScanResult preview={preview} /> : null;
  }
  if (toolName === "pr_blast") {
    const preview = parsePrBlast(content);
    return preview ? <PrBlastResult preview={preview} /> : null;
  }
  if (toolName === "dead_code") {
    const preview = parseDeadCode(content);
    return preview ? <DeadCodeResult preview={preview} /> : null;
  }
  return null;
}

function specializedMeta(
  toolName: string,
  content: string | null,
): string | undefined {
  if (!content) return undefined;
  if (toolName === "security_scan") {
    const preview = parseSecurityScan(content);
    return preview ? `${preview.findings} findings` : undefined;
  }
  if (toolName === "pr_blast") {
    const preview = parsePrBlast(content);
    if (!preview) return undefined;
    return `${preview.impactedFiles} files · risk ${formatScore(
      preview.riskScore,
    )}`;
  }
  if (toolName === "dead_code") {
    const preview = parseDeadCode(content);
    return preview ? `${preview.shown}/${preview.total} candidates` : undefined;
  }
  return undefined;
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
  const specialized = content ? (
    <SpecializedResult content={content} toolName={toolName} />
  ) : null;
  const meta = specializedMeta(toolName, content);

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

        {specialized}

        {content && !specialized && (
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
